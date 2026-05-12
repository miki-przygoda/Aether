use anyhow::{Context, Result};
use serde::Deserialize;

// ─── Domain types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default = "unknown_artist")]
    pub artist: String,
}

fn unknown_artist() -> String {
    "Unknown Artist".into()
}

// ─── Client ──────────────────────────────────────────────────────────────────

pub struct NavidromeClient {
    base_url: String,
    user: String,
    password: String,
    http: reqwest::Client,
}

impl NavidromeClient {
    pub fn new(
        base_url: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            user: user.into(),
            password: password.into(),
            http,
        }
    }

    /// MD5 token authentication as required by the Subsonic API.
    /// Returns (user, token, salt) for query-string inclusion.
    fn auth(&self) -> [(&'static str, String); 6] {
        // 8-char random hex salt generated from a random UUID
        let salt = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let token = format!("{:x}", md5::compute(format!("{}{}", self.password, salt)));
        [
            ("u", self.user.clone()),
            ("t", token),
            ("s", salt.to_string()),
            ("v", "1.16.1".into()),
            ("c", "aether".into()),
            ("f", "json".into()),
        ]
    }

    /// Return up to `count` random songs from the library.
    pub async fn get_random_songs(&self, count: u32) -> Result<Vec<Song>> {
        let auth = self.auth();
        let resp = self
            .http
            .get(format!("{}/rest/getRandomSongs", self.base_url))
            .query(&auth)
            .query(&[("size", count.to_string())])
            .send()
            .await
            .context("Navidrome: getRandomSongs request")?
            .json::<serde_json::Value>()
            .await
            .context("Navidrome: parsing getRandomSongs response")?;

        check_subsonic_status(&resp)?;
        Ok(extract_songs(&resp["subsonic-response"]["randomSongs"]["song"]))
    }

    /// Search for songs matching `query` via `search3`.
    pub async fn search_songs(&self, query: &str) -> Result<Vec<Song>> {
        let auth = self.auth();
        let resp = self
            .http
            .get(format!("{}/rest/search3", self.base_url))
            .query(&auth)
            .query(&[
                ("query", query),
                ("songCount", "5"),
                ("albumCount", "0"),
                ("artistCount", "0"),
            ])
            .send()
            .await
            .context("Navidrome: search3 request")?
            .json::<serde_json::Value>()
            .await
            .context("Navidrome: parsing search3 response")?;

        check_subsonic_status(&resp)?;
        Ok(extract_songs(&resp["subsonic-response"]["searchResult3"]["song"]))
    }

    /// Build an MP3 stream URL accessible from outside Docker (Pi on LAN).
    /// `external_base` is `http://{brain_ip}:{navidrome_port}`.
    pub fn stream_url(&self, song_id: &str, external_base: &str) -> String {
        let auth = self.auth();
        let [(_u, user), (_t, token), (_s, salt), ..] = auth;
        format!(
            "{}/rest/stream?u={}&t={}&s={}&v=1.16.1&c=aether&id={}&format=mp3",
            external_base, user, token, salt, song_id
        )
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn check_subsonic_status(resp: &serde_json::Value) -> Result<()> {
    let status = resp["subsonic-response"]["status"].as_str().unwrap_or("");
    if status != "ok" {
        let msg = resp["subsonic-response"]["error"]["message"]
            .as_str()
            .unwrap_or("unknown error");
        anyhow::bail!("Navidrome error: {msg}");
    }
    Ok(())
}

fn extract_songs(arr: &serde_json::Value) -> Vec<Song> {
    arr.as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|s| serde_json::from_value(s.clone()).ok())
        .collect()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> NavidromeClient {
        NavidromeClient::new(
            "http://localhost:4533",
            "admin",
            "password",
            reqwest::Client::new(),
        )
    }

    #[test]
    fn auth_produces_hex_token() {
        let c = client();
        let [(_u, _user), (_t, token), (_s, salt), ..] = c.auth();
        // token must be 32-char lowercase hex
        assert_eq!(token.len(), 32, "MD5 token should be 32 hex chars");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        // salt must be 8-char hex
        assert_eq!(salt.len(), 8);
    }

    #[test]
    fn auth_salts_differ_between_calls() {
        let c = client();
        // Run a few times — UUID-based salt is random so collisions are astronomically unlikely
        let salts: Vec<String> = (0..5).map(|_| {
            let [_, _, (_, s), ..] = c.auth();
            s
        }).collect();
        let unique: std::collections::HashSet<_> = salts.iter().collect();
        assert!(unique.len() > 1, "salts should differ across calls");
    }

    #[test]
    fn stream_url_contains_song_id() {
        let c = client();
        let url = c.stream_url("song-42", "http://192.168.1.5:4533");
        assert!(url.contains("id=song-42"), "URL should contain song ID");
        assert!(url.contains("format=mp3"), "URL should request MP3");
        assert!(url.starts_with("http://192.168.1.5:4533"), "should use external base");
    }

    #[test]
    fn extract_songs_handles_missing_fields() {
        let arr = serde_json::json!([
            { "id": "1", "title": "Song One", "artist": "Artist A" },
            { "id": "2", "title": "No Artist" },  // artist missing → default
            { "title": "No ID" },                  // id missing → filtered out
        ]);
        let songs = extract_songs(&arr);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].artist, "Artist A");
        assert_eq!(songs[1].artist, "Unknown Artist");
    }

    #[test]
    fn check_status_fails_on_error_response() {
        let resp = serde_json::json!({
            "subsonic-response": {
                "status": "failed",
                "error": { "code": 40, "message": "Wrong username or password." }
            }
        });
        assert!(check_subsonic_status(&resp).is_err());
    }

    #[test]
    fn check_status_ok_on_success() {
        let resp = serde_json::json!({
            "subsonic-response": { "status": "ok", "version": "1.16.1" }
        });
        assert!(check_subsonic_status(&resp).is_ok());
    }
}
