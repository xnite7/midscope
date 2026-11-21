//! Ranks, party guesses and smurf / boosted tags.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Rank {
    pub tier: u32,
    pub rr: u32,
    pub peak: u32,
    pub wins: u32,
    pub games: u32,
}

pub fn tier_name(t: u32) -> &'static str {
    const N: [&str; 28] = [
        "UNRANKED", "UNRANKED", "UNRANKED", "IRON 1", "IRON 2", "IRON 3", "BRONZE 1", "BRONZE 2",
        "BRONZE 3", "SILVER 1", "SILVER 2", "SILVER 3", "GOLD 1", "GOLD 2", "GOLD 3", "PLAT 1",
        "PLAT 2", "PLAT 3", "DIAMOND 1", "DIAMOND 2", "DIAMOND 3", "ASCENDANT 1", "ASCENDANT 2",
        "ASCENDANT 3", "IMMORTAL 1", "IMMORTAL 2", "IMMORTAL 3", "RADIANT",
    ];
    N.get(t as usize).copied().unwrap_or("UNRANKED")
}

/// Rank colour per tier group, matching the game's rank icons.
pub fn tier_color(t: u32) -> u32 {
    match t {
        3..=5 => 0x7C7C7C,
        6..=8 => 0xA5855E,
        9..=11 => 0xC9D2D3,
        12..=14 => 0xE8C25B,
        15..=17 => 0x55C6CF,
        18..=20 => 0xC487E9,
        21..=23 => 0x3DD598,
        24..=26 => 0xE5466A,
        27 => 0xFFF2B0,
        _ => 0x8B978F,
    }
}

/// Reads `/mmr/v1/players/{puuid}`.
pub fn parse_mmr(v: &Value) -> Rank {
    let latest = &v["LatestCompetitiveUpdate"];
    let season = latest["SeasonID"].as_str().unwrap_or("");
    let seasons = v.pointer("/QueueSkills/competitive/SeasonalInfoBySeasonID").and_then(Value::as_object);
    let current = seasons.and_then(|s| s.get(season));
    let num = |x: &Value| x.as_u64().unwrap_or(0) as u32;
    let peak = seasons
        .into_iter()
        .flat_map(|s| s.values())
        .flat_map(|s| s["WinsByTier"].as_object().into_iter().flatten())
        .filter_map(|(tier, _)| tier.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    let tier = current.map(|c| num(&c["CompetitiveTier"])).filter(|&t| t > 0).unwrap_or_else(|| num(&latest["TierAfterUpdate"]));
    Rank {
        tier,
        rr: current.map(|c| num(&c["RankedRating"])).unwrap_or(0),
        peak: peak.max(tier),
        wins: current.map(|c| num(&c["NumberOfWinsWithPlacements"]).max(num(&c["NumberOfWins"]))).unwrap_or(0),
        games: current.map(|c| num(&c["NumberOfGames"])).unwrap_or(0),
    }
}

/// Groups players into parties. `known` are certain groups (your own party,
/// from presence). Others are linked when they share two or more recent
/// matches: random matchmaking rarely puts two strangers together twice.
/// Returns a group number per player (0 = solo), groups of 2+ only.
pub fn parties(histories: &[(bool, &[String])], known: &[Vec<usize>], current: &str) -> Vec<u8> {
    let n = histories.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        if p[i] != i {
            let r = find(p, p[i]);
            p[i] = r;
        }
        p[i]
    }
    let union = |p: &mut Vec<usize>, a: usize, b: usize| {
        let (ra, rb) = (find(p, a), find(p, b));
        if ra != rb {
            p[ra] = rb;
        }
    };
    for g in known {
        for w in g.windows(2) {
            union(&mut parent, w[0], w[1]);
        }
    }
    let sets: Vec<HashSet<&str>> = histories
        .iter()
        .map(|(_, h)| h.iter().map(String::as_str).filter(|m| *m != current).collect())
        .collect();
    for a in 0..n {
        for b in a + 1..n {
            if histories[a].0 == histories[b].0 && sets[a].intersection(&sets[b]).count() >= 2 {
                union(&mut parent, a, b);
            }
        }
    }
    let roots: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let mut ids: Vec<(usize, u8)> = Vec::new();
    roots
        .iter()
        .map(|r| {
            if roots.iter().filter(|x| *x == r).count() < 2 {
                return 0;
            }
            if let Some((_, id)) = ids.iter().find(|(root, _)| root == r) {
                return *id;
            }
            let id = ids.len() as u8 + 1;
            ids.push((*r, id));
            id
        })
        .collect()
}

/// Colours for party 1, 2, 3... (midnite: lamp, neon-soft, good, plum light).
pub const PARTY_COLORS: [u32; 4] = [0xFFB25C, 0xFF6EC4, 0x7FD6A0, 0xC48AAE];

/// Tags shown next to a player. Kept to rules that are hard to hit by chance.
pub fn tags(level: u32, rank: &Rank, party_tiers: &[u32]) -> Vec<&'static str> {
    let mut out = Vec::new();
    let wr = if rank.games > 0 { rank.wins as f32 / rank.games as f32 } else { 0.0 };
    let fresh_high = level > 0 && level < 50 && rank.peak >= 15; // new account, Plat+ already
    let stomping = rank.games >= 10 && wr >= 0.70 && level < 120;
    if fresh_high || stomping {
        out.push("SMURF");
    }
    // Queued with someone 2+ ranks (6 tiers) higher: likely being carried.
    if rank.tier >= 3 && party_tiers.iter().any(|&t| t >= rank.tier + 6) {
        out.push("BOOSTED");
    }
    if rank.games >= 10 && wr <= 0.35 {
        out.push("TILTED");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_rank() {
        let v = json!({
            "LatestCompetitiveUpdate": { "SeasonID": "s2", "TierAfterUpdate": 19 },
            "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                "s1": { "CompetitiveTier": 22, "WinsByTier": { "21": 3, "22": 1 } },
                "s2": { "CompetitiveTier": 19, "RankedRating": 45, "NumberOfWins": 12, "NumberOfGames": 20, "WinsByTier": { "19": 2 } }
            }}}
        });
        let r = parse_mmr(&v);
        assert_eq!((r.tier, r.rr, r.peak, r.wins, r.games), (19, 45, 22, 12, 20));
        assert_eq!(tier_name(r.peak), "ASCENDANT 2");
    }

    #[test]
    fn links_players_who_keep_meeting() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = ids(&["now", "m1", "m2", "m3"]);
        let b = ids(&["now", "m1", "m3", "x"]);
        let c = ids(&["now", "m1", "y", "z"]); // met once: chance
        let d = ids(&["now", "q"]);
        let e = ids(&["now", "r"]);
        let hist = [(true, &a[..]), (true, &b[..]), (true, &c[..]), (true, &d[..]), (true, &e[..])];
        let p = parties(&hist, &[vec![3, 4]], "now");
        assert_eq!(p[0], p[1]);
        assert_ne!(p[0], 0);
        assert_eq!(p[2], 0);
        assert_eq!(p[3], p[4]);
        assert_ne!(p[3], p[0]);
    }

    #[test]
    fn tags_rules() {
        let r = Rank { tier: 18, peak: 18, wins: 8, games: 10, rr: 0 };
        assert_eq!(tags(30, &r, &[]), vec!["SMURF"]);
        let low = Rank { tier: 9, peak: 10, wins: 5, games: 10, rr: 0 };
        assert_eq!(tags(200, &low, &[18]), vec!["BOOSTED"]);
    }
}
