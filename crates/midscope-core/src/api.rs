//! Talks to the Riot Client's local API and, with its tokens, to Riot's
//! pd (player data) and glz (live game) servers. Read-only except for the
//! name-service lookup, which is a PUT by Riot's design.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::Value;

use crate::lockfile::{self, Lockfile};

#[derive(Debug)]
pub enum Error {
    NotRunning,
    Http(u16, String),
    Net(String),
    Bad(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => write!(f, "Riot Client isn't running"),
            Error::Http(c, u) => write!(f, "HTTP {c} from {u}"),
            Error::Net(e) => write!(f, "network: {e}"),
            Error::Bad(w) => write!(f, "unexpected reply: {w}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Session {
    pub puuid: String,
    pub access_token: String,
    pub entitlement: String,
    pub region: String, // na, eu, ap, kr, latam, br
    pub shard: String,  // na, eu, ap, kr
    pub client_version: String,
}

pub struct Riot {
    local: ureq::Agent,
    remote: ureq::Agent,
    lock: Lockfile,
    pub session: Session,
    platform: String,
}

impl Riot {
    /// Connects to a running Riot Client. Cheap enough to call on a retry loop.
    pub fn connect() -> Result<Riot> {
        let lock = lockfile::read().ok_or(Error::NotRunning)?;
        let local = local_agent();
        let remote = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(8))
            .user_agent("ShooterGame/13 Windows/10.0.19043.1.256.64bit")
            .build();
        let platform = base64::engine::general_purpose::STANDARD.encode(
            r#"{"platformType":"PC","platformOS":"Windows","platformOSVersion":"10.0.19042.1.256.64bit","platformChipset":"Unknown"}"#,
        );
        let mut riot = Riot {
            local,
            remote,
            lock,
            session: Session {
                puuid: String::new(),
                access_token: String::new(),
                entitlement: String::new(),
                region: String::new(),
                shard: String::new(),
                client_version: String::new(),
            },
            platform,
        };
        riot.refresh_tokens()?;
        riot.detect_region()?;
        riot.session.client_version = riot.fetch_version().unwrap_or_default();
        Ok(riot)
    }

    fn local_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.lock.port, path)
    }

    pub fn local_get(&self, path: &str) -> Result<Value> {
        let auth = base64::engine::general_purpose::STANDARD
            .encode(format!("riot:{}", self.lock.password));
        let url = self.local_url(path);
        call(self.local.get(&url).set("Authorization", &format!("Basic {auth}")), &url)
    }

    pub fn refresh_tokens(&mut self) -> Result<()> {
        let v = self.local_get("/entitlements/v1/token")?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        self.session.access_token = s("accessToken").ok_or(Error::Bad("accessToken"))?;
        self.session.entitlement = s("token").ok_or(Error::Bad("token"))?;
        self.session.puuid = s("subject").ok_or(Error::Bad("subject"))?;
        Ok(())
    }

    /// Region and shard come from VALORANT's launch arguments.
    fn detect_region(&mut self) -> Result<()> {
        let v = self.local_get("/product-session/v1/external-sessions")?;
        let args = v
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter(|s| s.get("productId").and_then(Value::as_str) == Some("valorant"))
            .flat_map(|s| s.pointer("/launchConfiguration/arguments").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let (region, shard) = parse_region(&args).ok_or(Error::Bad("region (start VALORANT)"))?;
        self.session.region = region;
        self.session.shard = shard;
        Ok(())
    }

    fn fetch_version(&self) -> Result<String> {
        let url = "https://valorant-api.com/v1/version";
        let v = call(self.remote.get(url), url)?;
        v.pointer("/data/riotClientVersion")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::Bad("version"))
    }

    fn pd(&self) -> String {
        format!("https://pd.{}.a.pvp.net", self.session.shard)
    }

    fn glz(&self) -> String {
        format!("https://glz-{}-1.{}.a.pvp.net", self.session.region, self.session.shard)
    }

    fn authed(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("Bearer {}", self.session.access_token))
            .set("X-Riot-Entitlements-JWT", &self.session.entitlement)
            .set("X-Riot-ClientPlatform", &self.platform)
            .set("X-Riot-ClientVersion", &self.session.client_version)
    }

    /// GET with one token refresh on 400/401 (tokens expire after about an hour).
    fn remote_get(&mut self, url: String) -> Result<Value> {
        match call(self.authed(self.remote.get(&url)), &url) {
            Err(Error::Http(400 | 401, _)) => {
                self.refresh_tokens()?;
                call(self.authed(self.remote.get(&url)), &url)
            }
            r => r,
        }
    }

    pub fn glz_get(&mut self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.glz(), path);
        self.remote_get(url)
    }

    pub fn pd_get(&mut self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.pd(), path);
        self.remote_get(url)
    }

    pub fn pd_put(&mut self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{}", self.pd(), path);
        let send = |r: &Riot| match r.authed(r.remote.put(&url)).send_json(body) {
            Ok(resp) => resp.into_json::<Value>().map_err(|e| Error::Net(e.to_string())),
            Err(ureq::Error::Status(c, _)) => Err(Error::Http(c, url.clone())),
            Err(e) => Err(Error::Net(e.to_string())),
        };
        match send(self) {
            Err(Error::Http(400 | 401, _)) => {
                self.refresh_tokens()?;
                send(self)
            }
            r => r,
        }
    }
}

fn call(req: ureq::Request, url: &str) -> Result<Value> {
    match req.call() {
        Ok(resp) => resp.into_json::<Value>().map_err(|e| Error::Net(e.to_string())),
        Err(ureq::Error::Status(c, _)) => Err(Error::Http(c, url.to_owned())),
        Err(e) => Err(Error::Net(e.to_string())),
    }
}

/// `-ares-deployment=eu` gives the region, `-config-endpoint=https://shared.eu.a.pvp.net` the shard.
pub fn parse_region(args: &[&str]) -> Option<(String, String)> {
    let get = |k: &str| args.iter().find_map(|a| a.strip_prefix(k)).map(str::to_owned);
    let region = get("-ares-deployment=")?;
    let shard = get("-config-endpoint=")
        .and_then(|u| u.split('.').nth(1).map(str::to_owned))
        .unwrap_or_else(|| match region.as_str() {
            "latam" | "br" => "na".into(),
            r => r.into(),
        });
    Some((region, shard))
}

/// The local API serves a self-signed certificate. This agent is only ever
/// pointed at 127.0.0.1, so certificate checks are skipped for it alone.
fn local_agent() -> ureq::Agent {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    #[derive(Debug)]
    struct Loopback(Arc<rustls::crypto::CryptoProvider>);
    impl ServerCertVerifier for Loopback {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn verify_tls13_signature(
            &self,
            m: &[u8],
            c: &CertificateDer<'_>,
            d: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Loopback(provider)))
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(4))
        .tls_config(Arc::new(config))
        .build()
}

#[cfg(test)]
mod tests {
    #[test]
    fn region() {
        let a = ["-ares-deployment=eu", "-config-endpoint=https://shared.eu.a.pvp.net"];
        assert_eq!(super::parse_region(&a), Some(("eu".into(), "eu".into())));
        let b = ["-ares-deployment=latam"];
        assert_eq!(super::parse_region(&b), Some(("latam".into(), "na".into())));
    }
}
