use scraper::{Html, Selector};
use serde::Serialize;
use std::time::Duration;

pub const HISTORY_URL: &str = "https://www.gtnewhorizons.com/version-history/";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub mode: String,
    pub java: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub channel: Channel,
    pub date: String,
    pub page_url: String,
    pub artifacts: Vec<Artifact>,
}

// Natural ordering, including RC-2 vs RC-10. Special/non-version releases are excluded.
fn version_key(value: &str) -> Option<(Vec<u32>, u8, Vec<u32>)> {
    let lower = value.to_ascii_lowercase();
    let mut split = lower.splitn(2, '-');
    let base: Vec<u32> = split
        .next()?
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if base.len() != 3 {
        return None;
    }
    let suffix = split.next().unwrap_or("");
    let (stage, remainder) = if suffix.is_empty() {
        (3, "")
    } else if let Some(rest) = suffix.strip_prefix("rc") {
        (2, rest)
    } else if let Some(rest) = suffix.strip_prefix("beta") {
        (1, rest)
    } else {
        return None;
    };
    let numbers = remainder
        .trim_start_matches('-')
        .split('-')
        .filter(|v| !v.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .ok()?;
    Some((base, stage, numbers))
}
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (version_key(candidate), version_key(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}
fn channel(version: &str) -> Option<Channel> {
    match version_key(version)?.1 {
        3 => Some(Channel::Stable),
        _ => Some(Channel::Beta),
    }
}

pub fn parse_history(html: &str) -> Result<Vec<Release>, String> {
    let document = Html::parse_document(html);
    let details = Selector::parse("details").unwrap();
    let summary = Selector::parse("summary").unwrap();
    let span = Selector::parse("span").unwrap();
    let anchors = Selector::parse("a[href]").unwrap();
    let mut releases = Vec::new();
    for block in document.select(&details) {
        let Some(header) = block.select(&summary).next() else {
            continue;
        };
        let Some(version) = header
            .select(&span)
            .next()
            .map(|s| s.text().collect::<String>().trim().to_string())
        else {
            continue;
        };
        let Some(mut release_channel) = channel(&version) else {
            continue;
        };
        // Historical releases can have a numeric tag but still be marked beta.
        if header.text().collect::<String>().contains("Beta release") {
            release_channel = Channel::Beta;
        }
        let date = header
            .select(&span)
            .map(|s| s.text().collect::<String>())
            .find(|text| {
                text.len() == 10 && text.as_bytes()[4] == b'/' && text.as_bytes()[7] == b'/'
            })
            .unwrap_or_default();
        let mut artifacts = Vec::new();
        let mut page_url = HISTORY_URL.to_string();
        for anchor in block.select(&anchors) {
            let href = anchor.value().attr("href").unwrap_or("");
            let Ok(url) = reqwest::Url::parse(href) else {
                continue;
            };
            if url.scheme() != "https" {
                continue;
            }
            if url.host_str() == Some("github.com") && url.path().contains("/changelogs/") {
                page_url = href.into();
            }
            if url.host_str() != Some("downloads.gtnewhorizons.com")
                || !url.path().ends_with(".zip")
            {
                continue;
            }
            let mode = if url.path().starts_with("/Multi_mc_downloads/") {
                "client"
            } else if url.path().starts_with("/ServerPacks/") {
                "server"
            } else {
                continue;
            };
            let text = anchor.text().collect::<String>();
            let java = text.trim().trim_end_matches(" ZIP").to_string();
            artifacts.push(Artifact {
                mode: mode.into(),
                java,
                url: href.into(),
            });
        }
        if !artifacts.is_empty() {
            releases.push(Release {
                version,
                channel: release_channel,
                date,
                page_url,
                artifacts,
            });
        }
    }
    releases.sort_by_key(|release| std::cmp::Reverse(version_key(&release.version)));
    releases.dedup_by(|a, b| a.version == b.version);
    if releases.is_empty() {
        return Err("No supported releases found in the official catalog".into());
    }
    Ok(releases)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("GTNH-Updater/0.1.0")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}
async fn text(url: &str) -> Result<String, String> {
    let response = client()?.get(url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Catalog HTTP {}.", response.status()));
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("Catalog response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
pub async fn fetch_official() -> Result<Vec<Release>, String> {
    parse_history(&text(HISTORY_URL).await?)
}

pub fn allowed_external_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some("github.com" | "www.gtnewhorizons.com" | "downloads.gtnewhorizons.com")
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a captured official page in GTNH_HISTORY_HTML"]
    fn captured_official_catalog() {
        let path = std::env::var("GTNH_HISTORY_HTML").expect("Set GTNH_HISTORY_HTML");
        let html = std::fs::read_to_string(path).unwrap();
        let releases = parse_history(&html).unwrap();
        assert!(releases.iter().any(|r| r.channel == Channel::Stable));
        assert!(releases.iter().any(|r| r.channel == Channel::Beta));
        assert!(releases.iter().all(|r| !r.artifacts.is_empty()));
        println!("Parsed {} official releases", releases.len());
    }
    #[test]
    fn versions_are_natural() {
        assert!(is_newer("2.9.0-RC-10", "2.9.0-RC-2"));
        assert!(is_newer("2.9.0", "2.9.0-RC-10"));
        assert!(is_newer("2.9.0-RC-1", "2.9.0-beta-3"));
        assert!(!is_newer("2.8.4", "2.9.0-beta-1"));
        assert!(channel("2.9.0-nightly-2026-10-24").is_none());
        assert!(channel("April fools 2025").is_none());
    }
    #[test]
    fn official_artifacts_are_filtered() {
        let fixture = r#"<details><summary><div><span>2.9.0-RC-2</span><span>Beta release</span></div><span>2026/10/04</span></summary>
          <a href="https://downloads.gtnewhorizons.com/Multi_mc_downloads/betas/pack.zip">Java 17-26 ZIP</a>
          <a href="https://downloads.gtnewhorizons.com/ServerPacks/pack.zip">Java 8 ZIP</a>
          <a href="https://evil.test/ServerPacks/pack.zip">Java 8 ZIP</a>
          <a href="https://downloads.gtnewhorizons.com/ClientPacks/pack.zip">Java 8 ZIP</a></details>
          <details><summary><span>April fools 2025</span></summary></details>"#;
        let releases = parse_history(fixture).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].artifacts.len(), 2);
        assert_eq!(releases[0].artifacts[0].java, "Java 17-26");
        assert_eq!(releases[0].channel, Channel::Beta);
        assert_eq!(releases[0].date, "2026/10/04");
        assert!(parse_history("<html>changed</html>").is_err());
    }
    #[test]
    fn external_urls_are_scoped() {
        assert!(allowed_external_url(
            "https://github.com/GTNewHorizons/test"
        ));
        assert!(!allowed_external_url("file:///etc/passwd"));
        assert!(!allowed_external_url("https://github.com.evil.test/test"));
    }
}
