//! Works out where you are (menus, agent select, in a match) and builds a
//! snapshot of every player: name, agent, level and equipped skins.

use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::{Result, Riot};
use crate::config::Config;
use crate::content::Content;
use crate::rank::{self, Rank};

const SOCKET_SKIN: &str = "bcef87d6-209b-46c6-8b19-fbe40bd95abc";
const SOCKET_BUDDY: &str = "dd3bf334-87f3-40bd-b043-682a57a8dc3a";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    Offline,
    Menus,
    Pregame,
    Ingame,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skin {
    pub weapon: String,
    pub skin: String,
    pub buddy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub puuid: String,
    pub name: String,
    /// The player is in streamer mode.
    pub incognito: bool,
    /// `name` is a real Riot ID that the game itself was hiding.
    pub revealed: bool,
    /// `name` came from name-service (not the agent stand-in).
    pub real_name: bool,
    pub ally: bool,
    pub is_self: bool,
    /// Position in Riot's roster; agent-select cards follow this order.
    pub slot: usize,
    pub agent: String,
    pub locked: bool,
    pub level: u32,
    pub skins: Vec<Skin>,
    pub rank: Option<Rank>,
    /// Party number (0 = solo) and how many are in it.
    pub party: u8,
    pub party_size: u8,
    pub tags: Vec<String>,
    /// Recent match ids, for party guessing.
    #[serde(skip)]
    pub history: Option<Vec<String>>,
}

impl Player {
    pub fn skin_for(&self, weapon: &str) -> Option<&Skin> {
        self.skins.iter().find(|s| s.weapon.eq_ignore_ascii_case(weapon))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub phase: Phase,
    pub match_id: String,
    pub players: Vec<Player>,
    /// Queue id from Riot ("competitive", "swiftplay", "deathmatch", ...).
    pub queue: String,
}

impl Snapshot {
    pub fn empty(phase: Phase) -> Snapshot {
        Snapshot { phase, match_id: String::new(), players: Vec::new(), queue: String::new() }
    }

    /// Free-for-all: no teams, six cards a side on the loading screen.
    pub fn is_deathmatch(&self) -> bool {
        self.queue == "deathmatch" || (self.phase == Phase::Ingame && self.players.len() > 10)
    }
}

/// Reads your own presence to find the game phase. One local call, very cheap.
pub fn phase(riot: &Riot) -> Result<Phase> {
    let v = riot.local_get("/chat/v4/presences")?;
    let me = v["presences"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["puuid"] == riot.session.puuid.as_str() && p["product"] == "valorant");
    let Some(me) = me else { return Ok(Phase::Offline) };
    let private = me["private"]
        .as_str()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(Value::Null);
    let state = private
        .pointer("/matchPresenceData/sessionLoopState")
        .or_else(|| private.get("sessionLoopState"))
        .and_then(Value::as_str)
        .unwrap_or("MENUS");
    Ok(match state {
        "PREGAME" => Phase::Pregame,
        "INGAME" => Phase::Ingame,
        _ => Phase::Menus,
    })
}

/// Builds a snapshot. Names and skins already known from `prev` (same match)
/// are reused, so each player is looked up once per match, not every poll.
/// Whether the match itself exists yet. Presence keeps saying "agent select"
/// through the VS loading screen; the live-game server already knows better.
pub fn in_core_game(riot: &mut Riot) -> bool {
    let me = riot.session.puuid.clone();
    riot.glz_get(&format!("/core-game/v1/players/{me}"))
        .map(|v| v["MatchID"].as_str().is_some_and(|m| !m.is_empty()))
        .unwrap_or(false)
}

pub fn snapshot(
    riot: &mut Riot,
    content: &Content,
    cfg: &Config,
    phase: Phase,
    prev: Option<&Snapshot>,
) -> Result<Snapshot> {
    let (base, roster_key) = match phase {
        Phase::Pregame => ("pregame", "/AllyTeam/Players"),
        Phase::Ingame => ("core-game", "/Players"),
        p => return Ok(Snapshot::empty(p)),
    };
    let me = riot.session.puuid.clone();
    let mid = riot.glz_get(&format!("/{base}/v1/players/{me}"))?["MatchID"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if mid.is_empty() {
        return Ok(Snapshot::empty(phase));
    }
    let m = riot.glz_get(&format!("/{base}/v1/matches/{mid}"))?;
    let roster: Vec<Value> = m.pointer(roster_key).and_then(Value::as_array).cloned().unwrap_or_default();
    let my_team = roster
        .iter()
        .find(|p| p["Subject"] == me.as_str())
        .and_then(|p| p["TeamID"].as_str())
        .unwrap_or("")
        .to_owned();

    let mut players: Vec<Player> = roster
        .iter()
        .enumerate()
        .map(|(slot, p)| {
            let puuid = p["Subject"].as_str().unwrap_or_default().to_owned();
            let ident = &p["PlayerIdentity"];
            let agent = content.name(p["CharacterID"].as_str().unwrap_or("")).unwrap_or("").to_owned();
            Player {
                is_self: puuid == me,
                slot,
                ally: phase == Phase::Pregame || p["TeamID"].as_str() == Some(&my_team),
                incognito: ident["Incognito"].as_bool().unwrap_or(false),
                revealed: false,
                real_name: false,
                level: ident["AccountLevel"].as_u64().unwrap_or(0) as u32,
                locked: p["CharacterSelectionState"] == "locked",
                name: String::new(),
                agent,
                puuid,
                skins: Vec::new(),
                rank: None,
                party: 0,
                party_size: 0,
                tags: Vec::new(),
                history: None,
            }
        })
        .collect();

    // Same match, or agent select carrying over into the match it started.
    let prev = prev.filter(|s| s.match_id == mid || (s.phase == Phase::Pregame && phase == Phase::Ingame));
    if let Some(prev) = prev {
        for p in players.iter_mut() {
            if let Some(old) = prev.players.iter().find(|o| o.puuid == p.puuid) {
                if old.real_name {
                    p.name.clone_from(&old.name);
                    p.revealed = old.revealed;
                    p.real_name = true;
                }
                p.skins.clone_from(&old.skins);
                p.rank.clone_from(&old.rank);
                p.history.clone_from(&old.history);
                p.party = old.party;
                p.party_size = old.party_size;
                p.tags.clone_from(&old.tags);
            }
        }
    }
    fill_names(riot, cfg, &mut players);
    if players.iter().any(|p| p.skins.is_empty()) {
        if let Ok(l) = riot.glz_get(&format!("/{base}/v1/matches/{mid}/loadouts")) {
            fill_skins(&l, content, &mut players);
        }
    }
    // Ranks and parties are filled in afterwards by `enrich`, so the match
    // can be shown straight away.
    players.sort_by_key(|p| (!p.ally, p.slot));
    let queue = m
        .pointer("/MatchmakingData/QueueID")
        .or_else(|| m.get("QueueID"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok(Snapshot { phase, match_id: mid, players, queue })
}

/// Asks name-service for Riot IDs. Streamer-mode players are only asked for
/// when the user turned `reveal_hidden_names` on; otherwise they keep their
/// agent's name, exactly as the game shows them.
fn fill_names(riot: &mut Riot, cfg: &Config, players: &mut [Player]) {
    let asked: Vec<&str> = players
        .iter()
        .filter(|p| !p.real_name)
        .filter(|p| !p.incognito || p.is_self || cfg.reveal_hidden_names)
        .map(|p| p.puuid.as_str())
        .collect();
    let rows = if asked.is_empty() {
        Value::Null
    } else {
        riot.pd_put("/name-service/v2/players", &json!(asked)).unwrap_or(Value::Null)
    };
    for p in players.iter_mut().filter(|p| !p.real_name) {
        let found = rows.as_array().into_iter().flatten().find(|r| r["Subject"] == p.puuid.as_str());
        let id = found.and_then(|r| {
            let g = r["GameName"].as_str().filter(|s| !s.is_empty())?;
            Some(format!("{g}#{}", r["TagLine"].as_str().unwrap_or("")))
        });
        match id {
            Some(id) => {
                p.revealed = p.incognito && !p.is_self;
                p.real_name = true;
                p.name = id;
            }
            None => p.name = if p.agent.is_empty() { "Player".into() } else { p.agent.clone() },
        }
    }
}

/// Ranks and recent matches, fetched once per player per match, then
/// party grouping and tags.
/// Whether some player still lacks a rank or match history.
pub fn needs_enrich(s: &Snapshot) -> bool {
    s.players.iter().any(|p| p.rank.is_none() || p.history.is_none())
}

/// Fills in ranks, recent matches, parties and tags. Every player is looked
/// up at the same time (one thread each), so ten players take about as long
/// as one.
pub fn enrich(riot: &Riot, snap: &mut Snapshot) {
    let mid = snap.match_id.clone();
    let todo: Vec<(usize, String, bool, bool)> = snap
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| p.rank.is_none() || p.history.is_none())
        .map(|(i, p)| (i, p.puuid.clone(), p.rank.is_none(), p.history.is_none()))
        .collect();
    let results: Vec<(usize, Option<Rank>, Option<Vec<String>>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = todo
            .into_iter()
            .map(|(i, puuid, want_rank, want_hist)| {
                let mut r = riot.clone();
                scope.spawn(move || {
                    let rank = want_rank
                        .then(|| r.pd_get(&format!("/mmr/v1/players/{puuid}")).ok().map(|v| rank::parse_mmr(&v)))
                        .flatten();
                    let hist = want_hist
                        .then(|| {
                            r.pd_get(&format!("/match-history/v1/history/{puuid}?startIndex=0&endIndex=10")).ok().map(|v| {
                                v["History"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|m| m["MatchID"].as_str().map(str::to_owned))
                                    .collect::<Vec<_>>()
                            })
                        })
                        .flatten();
                    (i, rank, hist)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (i, rank, hist) in results {
        let p = &mut snap.players[i];
        if rank.is_some() {
            p.rank = rank;
        }
        if hist.is_some() {
            p.history = hist;
        }
    }
    fill_ranks_and_parties(riot, &mid, &mut snap.players);
}

fn fill_ranks_and_parties(riot: &Riot, mid: &str, players: &mut [Player]) {
    let mine = own_party(riot);
    let known: Vec<Vec<usize>> = vec![players
        .iter()
        .enumerate()
        .filter(|(_, p)| mine.contains(&p.puuid))
        .map(|(i, _)| i)
        .collect()];
    let empty: Vec<String> = Vec::new();
    let hist: Vec<(bool, &[String])> =
        players.iter().map(|p| (p.ally, p.history.as_deref().unwrap_or(&empty))).collect();
    let groups = rank::parties(&hist, &known, mid);
    for (i, g) in groups.iter().enumerate() {
        players[i].party = *g;
        players[i].party_size = if *g == 0 { 0 } else { groups.iter().filter(|x| *x == g).count() as u8 };
    }
    let snapshot: Vec<(u8, u32)> = players.iter().map(|p| (p.party, p.rank.as_ref().map_or(0, |r| r.tier))).collect();
    for (i, p) in players.iter_mut().enumerate() {
        let mates: Vec<u32> = snapshot
            .iter()
            .enumerate()
            .filter(|(j, (g, _))| *j != i && *g != 0 && *g == p.party)
            .map(|(_, (_, t))| *t)
            .collect();
        p.tags = p.rank.as_ref().map(|r| rank::tags(p.level, r, &mates)).unwrap_or_default().into_iter().map(String::from).collect();
    }
}

/// Your own party, from presences (exact, no guessing).
fn own_party(riot: &Riot) -> Vec<String> {
    let Ok(v) = riot.local_get("/chat/v4/presences") else { return Vec::new() };
    let decode = |p: &Value| {
        p["private"]
            .as_str()
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or(Value::Null)
    };
    let party_of = |v: &Value| {
        v.pointer("/partyPresenceData/partyId").or_else(|| v.get("partyId")).and_then(Value::as_str).unwrap_or("").to_owned()
    };
    let list: Vec<&Value> = v["presences"].as_array().into_iter().flatten().filter(|p| p["product"] == "valorant").collect();
    let Some(me) = list.iter().find(|p| p["puuid"] == riot.session.puuid.as_str()) else { return Vec::new() };
    let mine = party_of(&decode(me));
    if mine.is_empty() {
        return Vec::new();
    }
    list.iter()
        .filter(|p| party_of(&decode(p)) == mine)
        .filter_map(|p| p["puuid"].as_str().map(str::to_owned))
        .collect()
}

/// Pregame loadouts come in roster order; core-game ones carry a Subject.
fn fill_skins(l: &Value, content: &Content, players: &mut [Player]) {
    let list = l["Loadouts"].as_array().cloned().unwrap_or_default();
    for (i, entry) in list.iter().enumerate() {
        let lo = entry.get("Loadout").unwrap_or(entry);
        let owner = lo["Subject"].as_str().map(str::to_owned).or_else(|| players.get(i).map(|p| p.puuid.clone()));
        let Some(p) = owner.and_then(|o| players.iter_mut().find(|p| p.puuid == o)) else { continue };
        p.skins = skins_of(&lo["Items"], content);
    }
}

pub fn skins_of(items: &Value, content: &Content) -> Vec<Skin> {
    let Some(items) = items.as_object() else { return Vec::new() };
    let mut out: Vec<Skin> = items
        .iter()
        .filter_map(|(weapon_id, it)| {
            let socket = |s: &str| it.pointer(&format!("/Sockets/{s}/Item/ID")).and_then(Value::as_str);
            let weapon = content.name(weapon_id)?.to_owned();
            let skin = socket(SOCKET_SKIN).and_then(|id| content.name(id)).unwrap_or("").to_owned();
            let skin = if skin.is_empty() || skin.starts_with("Standard") { "Default".into() } else { skin };
            let buddy = socket(SOCKET_BUDDY).and_then(|id| content.name(id)).map(str::to_owned);
            Some(Skin { weapon, skin, buddy })
        })
        .collect();
    out.sort_by(|a, b| a.weapon.cmp(&b.weapon));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_skins() {
        let mut c = Content::default();
        c.names.insert("w1".into(), "Vandal".into());
        c.names.insert("s1".into(), "Prime Vandal".into());
        c.names.insert("b1".into(), "Spike Buddy".into());
        let items = json!({ "W1": { "Sockets": {
            SOCKET_SKIN: { "Item": { "ID": "S1" } },
            SOCKET_BUDDY: { "Item": { "ID": "b1" } }
        }}});
        let s = skins_of(&items, &c);
        assert_eq!(s[0].weapon, "Vandal");
        assert_eq!(s[0].skin, "Prime Vandal");
        assert_eq!(s[0].buddy.as_deref(), Some("Spike Buddy"));
    }
}
