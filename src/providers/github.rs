use crate::{
    errors::{SkillsError, SkillsResult},
    models::SkillEntry,
    providers::{ExtractTarget, InstallPlan, ResolvedSkill, SkillProvider},
};
use flate2::read::GzDecoder;
use regex::Regex;
use rustls_pki_types::CertificateDer;
use serde::Deserialize;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::OnceLock;
use std::{env, fs};
use tar::Archive;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, parse_pem};
use ureq::typestate::WithoutBody;
use ureq::{RequestBuilder, config::Config};

const GITHUB_API_VERSION: &str = "2026-03-10";

#[derive(Debug, Clone)]
pub struct GitHubUrlSpec {
    pub slug: String,
    pub tail: Vec<String>,
}

impl GitHubUrlSpec {
    pub fn parse(url: &str) -> SkillsResult<Self> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"^https://github\.com/([^/]+/[^/]+)/tree/(.+?)/?$").unwrap()
        });

        let url = url.trim_end_matches('/');
        let captures = re
            .captures(url)
            .ok_or_else(|| SkillsError::InvalidUrl(url.to_string()))?;

        let tail: Vec<String> = captures[2]
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.to_string())
            .collect();

        if tail.len() < 2 {
            return SkillsError::InvalidUrl(url.to_string()).into();
        }

        Ok(Self {
            slug: captures[1].to_string(),
            tail,
        })
    }

    pub fn directory_name(&self) -> &str {
        self.tail.last().map(String::as_str).unwrap()
    }

    pub fn candidates(&self) -> Vec<GitHubUrl> {
        (1..self.tail.len())
            .map(|split| {
                let r#ref = self.tail[..split].join("/");
                GitHubUrl {
                    slug: self.slug.clone(),
                    sha: r#ref.clone(),
                    r#ref,
                    path: self.tail[split..].join("/"),
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct GitHubUrl {
    pub slug: String,
    pub r#ref: String,
    pub sha: String,
    pub path: String,
}

impl GitHubUrl {
    pub fn with_sha(mut self, sha: String) -> Self {
        self.sha = sha;
        self
    }

    pub fn child(&self, child_name: &str) -> Self {
        Self {
            slug: self.slug.clone(),
            r#ref: self.sha.clone(),
            sha: self.sha.clone(),
            path: format!("{}/{}", self.path, child_name),
        }
    }

    pub fn tarball_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/tarball/{}",
            self.slug,
            urlencoding::encode(&self.sha)
        )
    }

    pub fn commits_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/commits?sha={}&path={}&per_page=1",
            self.slug,
            urlencoding::encode(&self.r#ref),
            urlencoding::encode(&self.path)
        )
    }

    pub fn contents_url(&self) -> String {
        fn encode_path_segments(path: &str) -> String {
            path.split('/')
                .map(|part| urlencoding::encode(part).into_owned())
                .collect::<Vec<_>>()
                .join("/")
        }
        format!(
            "https://api.github.com/repos/{}/contents/{}?ref={}",
            self.slug,
            encode_path_segments(&self.path),
            urlencoding::encode(&self.sha)
        )
    }
}

fn proxy_from_env() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

const TLS_ROOTS_ENV: &str = "SKILLS_MAN_TLS_ROOTS";
const CA_BUNDLE_ENV: &str = "SSL_CERT_FILE";
const CA_BUNDLE_DIR_ENV: &str = "SSL_CERT_DIR";

fn root_certs_from_env() -> SkillsResult<RootCerts> {
    let strategy = env::var(TLS_ROOTS_ENV).ok();
    let bundle = env::var_os(CA_BUNDLE_ENV);
    let bundle_dirs = env::var_os(CA_BUNDLE_DIR_ENV);
    resolve_root_certs(
        strategy.as_deref(),
        bundle.as_deref(),
        bundle_dirs.as_deref(),
    )
}

// Trust anchors default to the Mozilla roots bundled into the binary: the same
// build then trusts the same CAs on every machine, and a TLS-intercepting proxy
// cannot slip in unnoticed. Both ways out are opt-in, never a fallback after a
// failed handshake, so widening trust is always a deliberate act.
//
// The two ways out are mutually exclusive rather than ranked. A precedence rule
// could not be honoured anyway: on unix the platform verifier resolves its roots
// through rustls-native-certs, which reads SSL_CERT_FILE/SSL_CERT_DIR *instead*
// of the OS store whenever either is set, so `system` plus a bundle would mean
// different things on Linux and macOS.
fn resolve_root_certs(
    strategy: Option<&str>,
    bundle: Option<&OsStr>,
    bundle_dirs: Option<&OsStr>,
) -> SkillsResult<RootCerts> {
    let strategy = match strategy.map(str::trim).filter(|value| !value.is_empty()) {
        Some(strategy) => strategy,
        // No strategy: the CA variables have to be read here. The bundled roots
        // ignore them entirely and the platform verifier only honours them on
        // unix, so this is what gives them the same meaning on every platform.
        None => return root_certs_from_bundles(bundle, bundle_dirs),
    };

    let roots = match strategy {
        // What curl and git trust, including any CA a corporate or sandbox proxy
        // installed there.
        "system" => RootCerts::PlatformVerifier,
        "webpki" => RootCerts::WebPki,
        other => return SkillsError::InvalidTlsRoots(other.to_string()).into(),
    };

    // Presence, not content. On unix rustls-native-certs branches on whether the
    // variable exists at all, so even SSL_CERT_FILE= (set but empty) diverts it
    // away from the OS store and would leave `system` with no roots whatsoever.
    let mut conflicting = Vec::new();
    if bundle.is_some() {
        conflicting.push(CA_BUNDLE_ENV.to_string());
    }
    if bundle_dirs.is_some() {
        conflicting.push(CA_BUNDLE_DIR_ENV.to_string());
    }
    if !conflicting.is_empty() {
        return SkillsError::ConflictingTlsConfig {
            strategy: strategy.to_string(),
            vars: conflicting,
        }
        .into();
    }

    Ok(roots)
}

// Both CA variables follow the OpenSSL convention that rustls-native-certs also
// implements, so `system` and the default path agree on what they mean: a PEM
// file, plus any number of directories of PEM files, unioned together.
fn root_certs_from_bundles(
    bundle: Option<&OsStr>,
    bundle_dirs: Option<&OsStr>,
) -> SkillsResult<RootCerts> {
    // Here an empty value really is "not configured": without a strategy the
    // roots come from these variables alone, and an empty one names no path.
    let bundle = bundle.filter(|value| !value.is_empty());
    let bundle_dirs = bundle_dirs.filter(|value| !value.is_empty());
    if bundle.is_none() && bundle_dirs.is_none() {
        return Ok(RootCerts::WebPki);
    }

    let mut loaded = LoadedCerts::default();
    let mut sources = Vec::new();

    if let Some(path) = bundle {
        let path = Path::new(path);
        sources.push(path.display().to_string());
        let pem = fs::read(path).map_err(|e| SkillsError::InvalidCaBundle {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        loaded.add_pem(&pem);
    }

    if let Some(dirs) = bundle_dirs {
        for dir in env::split_paths(dirs) {
            sources.push(dir.display().to_string());
            let entries = fs::read_dir(&dir).map_err(|e| SkillsError::InvalidCaBundle {
                path: dir.display().to_string(),
                reason: e.to_string(),
            })?;
            // Unreadable entries are skipped rather than fatal: these directories
            // routinely hold unrelated files, and OpenSSL's c_rehash layout fills
            // them with symlinks.
            for entry in entries.flatten() {
                if let Ok(pem) = fs::read(entry.path()) {
                    loaded.add_pem(&pem);
                }
            }
        }
    }

    if loaded.certs.is_empty() {
        return SkillsError::InvalidCaBundle {
            path: sources.join(", "),
            reason: match loaded.unusable {
                0 => "no certificate found".to_string(),
                1 => "the only certificate found is not a usable trust anchor".to_string(),
                n => format!("none of the {n} certificates found is a usable trust anchor"),
            },
        }
        .into();
    }
    Ok(loaded.certs.into())
}

#[derive(Default)]
struct LoadedCerts {
    certs: Vec<Certificate<'static>>,
    unusable: usize,
}

impl LoadedCerts {
    fn add_pem(&mut self, pem: &[u8]) {
        for item in parse_pem(pem).flatten() {
            let PemItem::Certificate(cert) = item else {
                continue;
            };
            // parse_pem checks only the PEM framing and its base64, so without
            // decoding the DER too, ureq would silently drop the certificate
            // later and a broken file would surface as a handshake failure.
            let der = CertificateDer::from(cert.der());
            match webpki::anchor_from_trusted_cert(&der) {
                Ok(_) => self.certs.push(cert),
                Err(_) => self.unusable += 1,
            }
        }
    }
}

fn github_token_from_env() -> Option<String> {
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(value) = env::var(key)
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

fn config_github_request(request: RequestBuilder<WithoutBody>) -> RequestBuilder<WithoutBody> {
    let mut request = request
        .header("User-Agent", "skills-man")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION);
    if let Some(token) = github_token_from_env() {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    request
}

#[derive(Debug)]
enum SkillDetectionResult {
    Single,
    Batch(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct ContentItem {
    name: String,
    #[serde(rename = "type")]
    item_type: String,
}

pub struct GitHubProvider {
    agent: ureq::Agent,
}

impl GitHubProvider {
    pub fn new() -> SkillsResult<Self> {
        let mut builder = Config::builder().tls_config(
            TlsConfig::builder()
                .root_certs(root_certs_from_env()?)
                .build(),
        );

        if let Some(proxy_url) = proxy_from_env() {
            let proxy = ureq::Proxy::new(&proxy_url)
                .map_err(|e| SkillsError::NetworkError(e.to_string()))?;
            builder = builder.proxy(Some(proxy));
        }

        Ok(Self {
            agent: ureq::Agent::new_with_config(builder.build()),
        })
    }

    fn download_and_extract(&self, url: &str, targets: &[ExtractTarget]) -> SkillsResult<()> {
        let response = match config_github_request(self.agent.get(url))
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(status)) => {
                return Err(match status {
                    404 => SkillsError::NotFound {
                        url: url.to_string(),
                    },
                    403 => SkillsError::Forbidden {
                        url: url.to_string(),
                    },
                    429 => SkillsError::RateLimited,
                    _ => SkillsError::HttpError {
                        status,
                        message: url.to_string(),
                    },
                });
            }
            Err(e) => return SkillsError::transport(e.to_string()).into(),
        };

        let decoder = GzDecoder::new(response.into_body().into_reader());
        let mut archive = Archive::new(decoder);

        let mut found = vec![false; targets.len()];
        let mut top_level_dir: Option<String> = None;

        for entry in archive
            .entries()
            .map_err(|e| SkillsError::InvalidArchive(e.to_string()))?
        {
            let mut entry = entry.map_err(|e| SkillsError::InvalidArchive(e.to_string()))?;
            let entry_path = entry
                .path()
                .map_err(|e| SkillsError::InvalidArchive(e.to_string()))?;
            let entry_str = entry_path.to_string_lossy();

            if top_level_dir.is_none()
                && let Some(slash_pos) = entry_str.find('/')
            {
                top_level_dir = Some(entry_str[..slash_pos].to_string());
            }

            if let Some(ref top_dir) = top_level_dir {
                for (idx, target) in targets.iter().enumerate() {
                    let expected_prefix = format!("{}/{}/", top_dir, target.path);
                    if entry_str.starts_with(&expected_prefix) {
                        let relative = &entry_str[expected_prefix.len()..];
                        if relative.is_empty() {
                            break;
                        }
                        found[idx] = true;
                        let dest_path = target.dest_dir.join(relative);
                        if let Some(parent) = dest_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        entry.unpack(&dest_path)?;
                        break;
                    }
                }
            }
        }

        let missing_paths: Vec<_> = targets
            .iter()
            .zip(found)
            .filter_map(|(target, found)| (!found).then_some(target.path.clone()))
            .collect();

        if !missing_paths.is_empty() {
            return SkillsError::PathNotFound(missing_paths).into();
        }

        Ok(())
    }

    fn resolve_commit_sha(&self, github_url: &GitHubUrl) -> SkillsResult<Option<String>> {
        let url = github_url.commits_url();
        match config_github_request(self.agent.get(&url))
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(response) => {
                let json: serde_json::Value = response
                    .into_body()
                    .read_json()
                    .map_err(|e| SkillsError::transport(e.to_string()))?;
                let sha = json
                    .get(0)
                    .and_then(|x| x.get("sha"))
                    .and_then(|x| x.as_str());
                let Some(sha) = sha else {
                    return Ok(None);
                };
                Ok(Some(sha.to_string()))
            }
            Err(ureq::Error::StatusCode(status)) => match status {
                404 | 422 => Ok(None),
                403 => Err(SkillsError::Forbidden { url }),
                429 => Err(SkillsError::RateLimited),
                _ => Err(SkillsError::HttpError {
                    status,
                    message: url,
                }),
            },
            Err(e) => Err(SkillsError::transport(e.to_string())),
        }
    }

    fn resolve(&self, spec: &GitHubUrlSpec) -> SkillsResult<Option<GitHubUrl>> {
        for candidate in spec.candidates() {
            let sha = self.resolve_commit_sha(&candidate)?;
            if let Some(sha) = sha {
                return Ok(Some(candidate.with_sha(sha)));
            }
        }
        Ok(None)
    }

    fn list_directory_contents(&self, github_url: &GitHubUrl) -> SkillsResult<Vec<ContentItem>> {
        let url = github_url.contents_url();

        match config_github_request(self.agent.get(&url))
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(response) => {
                let items: Vec<ContentItem> = response
                    .into_body()
                    .read_json()
                    .map_err(|e| SkillsError::transport(e.to_string()))?;
                Ok(items)
            }
            Err(ureq::Error::StatusCode(status)) => match status {
                404 => Err(SkillsError::PathNotFound(vec![github_url.path.clone()])),
                403 => Err(SkillsError::Forbidden { url }),
                429 => Err(SkillsError::RateLimited),
                _ => Err(SkillsError::HttpError {
                    status,
                    message: url,
                }),
            },
            Err(e) => Err(SkillsError::transport(e.to_string())),
        }
    }

    fn detect_skill_type(&self, github_url: &GitHubUrl) -> SkillsResult<SkillDetectionResult> {
        let contents = self.list_directory_contents(github_url)?;

        let has_skill_manifest = contents
            .iter()
            .any(|item| item.item_type == "file" && item.name.eq_ignore_ascii_case("SKILL.md"));

        if has_skill_manifest {
            return Ok(SkillDetectionResult::Single);
        }

        let subdirs: Vec<&ContentItem> = contents
            .iter()
            .filter(|item| item.item_type == "dir")
            .collect();

        let mut skill_dirs = Vec::new();

        for subdir in subdirs {
            let child_url = GitHubUrl {
                slug: github_url.slug.clone(),
                r#ref: github_url.r#ref.clone(),
                sha: github_url.sha.clone(),
                path: format!("{}/{}", github_url.path, subdir.name),
            };

            let child_contents = self.list_directory_contents(&child_url)?;
            let has_skill = child_contents
                .iter()
                .any(|item| item.item_type == "file" && item.name.eq_ignore_ascii_case("SKILL.md"));

            if has_skill {
                skill_dirs.push(subdir.name.clone());
            }
        }

        if skill_dirs.is_empty() {
            return Err(SkillsError::NoSkillsFound(github_url.path.clone()));
        }

        Ok(SkillDetectionResult::Batch(skill_dirs))
    }
}

impl SkillProvider for GitHubProvider {
    fn handles(&self, url: &str) -> bool {
        url.starts_with("https://github.com/")
    }

    /// Parse a GitHub tree URL, resolve refs to SHAs via the commits API,
    /// detect single vs batch skill layout via the contents API, and return
    /// an [`InstallPlan`] with a tarball URL for the resolved SHA.
    fn resolve_install_plan(&self, url: &str) -> SkillsResult<InstallPlan> {
        let source_url = url.trim_end_matches('/');
        let spec = GitHubUrlSpec::parse(source_url)?;

        let Some(resolved) = self.resolve(&spec)? else {
            return Err(SkillsError::PathNotFound(vec![source_url.to_string()]));
        };

        let plan = match self.detect_skill_type(&resolved)? {
            SkillDetectionResult::Single => InstallPlan {
                archive_url: resolved.tarball_url(),
                is_batch: false,
                skills: vec![ResolvedSkill {
                    name: spec.directory_name().to_string(),
                    source_url: source_url.to_string(),
                    collection_url: None,
                    slug: resolved.slug,
                    sha: resolved.sha,
                    path: resolved.path,
                }],
            },
            SkillDetectionResult::Batch(subdirs) => {
                let mut skills = Vec::new();
                for subdir in subdirs {
                    let child_source_url = format!("{}/{}", source_url, subdir);
                    let child_candidate = resolved.child(&subdir);
                    let Some(child_sha) = self.resolve_commit_sha(&child_candidate)? else {
                        return Err(SkillsError::PathNotFound(vec![child_source_url]));
                    };
                    let child_resolved = child_candidate.with_sha(child_sha);

                    skills.push(ResolvedSkill {
                        name: subdir,
                        source_url: child_source_url,
                        collection_url: Some(source_url.to_string()),
                        slug: child_resolved.slug,
                        sha: child_resolved.sha,
                        path: child_resolved.path,
                    });
                }

                InstallPlan {
                    archive_url: resolved.tarball_url(),
                    is_batch: true,
                    skills,
                }
            }
        };

        Ok(plan)
    }

    fn fetch_and_extract(&self, archive_url: &str, targets: &[ExtractTarget]) -> SkillsResult<()> {
        self.download_and_extract(archive_url, targets)
    }

    fn archive_url_for_entry(&self, entry: &SkillEntry) -> String {
        format!(
            "https://api.github.com/repos/{}/tarball/{}",
            entry.slug,
            urlencoding::encode(&entry.sha)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_url_basic() {
        let url = "https://github.com/anthropics/skills/tree/main/skills/frontend-design";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "anthropics/skills");
        assert_eq!(result.tail, vec!["main", "skills", "frontend-design"]);
    }

    #[test]
    fn test_parse_valid_url_commit_hash() {
        let url = "https://github.com/owner/repo/tree/00756142ab04c82a447693cf373c4e0c554d1005/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(
            result.tail,
            vec![
                "00756142ab04c82a447693cf373c4e0c554d1005",
                "path",
                "to",
                "dir"
            ]
        );
    }

    #[test]
    fn test_parse_valid_url_trailing_slash() {
        let url = "https://github.com/owner/repo/tree/main/path/";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(result.tail, vec!["main", "path"]);
    }

    #[test]
    fn test_parse_valid_url_ref_with_slash() {
        let url = "https://github.com/owner/repo/tree/feature/foo/path/to/dir";
        let result = GitHubUrlSpec::parse(url).unwrap();

        assert_eq!(result.slug, "owner/repo");
        assert_eq!(result.tail, vec!["feature", "foo", "path", "to", "dir"]);
    }

    #[test]
    fn test_candidates_include_slash_ref() {
        let candidates =
            GitHubUrlSpec::parse("https://github.com/owner/repo/tree/release/v1.0/hotfix/skill")
                .unwrap()
                .candidates();
        assert!(candidates.iter().any(|candidate| {
            candidate.r#ref == "release/v1.0" && candidate.path == "hotfix/skill"
        }));
    }

    #[test]
    fn test_parse_invalid_url_missing_tree() {
        let url = "https://github.com/owner/repo";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_parse_invalid_url_wrong_protocol() {
        let url = "http://github.com/owner/repo/tree/main/path";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_parse_invalid_url_missing_path() {
        let url = "https://github.com/owner/repo/tree/main";
        let result = GitHubUrlSpec::parse(url);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SkillsError::InvalidUrl(_)));
    }

    #[test]
    fn test_directory_name() {
        let github_url = GitHubUrlSpec {
            slug: "owner/repo".to_string(),
            tail: vec![
                "main".to_string(),
                "skills".to_string(),
                "frontend-design".to_string(),
            ],
        };

        assert_eq!(github_url.directory_name(), "frontend-design");
    }

    #[test]
    fn test_directory_name_single_component() {
        let github_url = GitHubUrlSpec {
            slug: "owner/repo".to_string(),
            tail: vec!["main".to_string(), "skill".to_string()],
        };

        assert_eq!(github_url.directory_name(), "skill");
    }

    // Self-signed EC certificate, only ever used as a parseable trust anchor.
    // anchor_from_trusted_cert does no time validation, so it cannot expire out
    // from under these tests.
    const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBkDCCATegAwIBAgIUN8uAwtCoPFgH+V3UmLsaijUZ8o4wCgYIKoZIzj0EAwIw\n\
HTEbMBkGA1UEAwwSc2tpbGxzLW1hbiB0ZXN0IENBMCAXDTI2MDgwNDEzNDQwN1oY\n\
DzIxMjYwNzExMTM0NDA3WjAdMRswGQYDVQQDDBJza2lsbHMtbWFuIHRlc3QgQ0Ew\n\
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAR4F+5CPIGPqUmPcfVDPePvq+MQsrrm\n\
vtphiaRWoTdEQ2OxKTQLGdeKtdPyrLNmhz+GTqp5k72X6c2Q7O54SsLio1MwUTAd\n\
BgNVHQ4EFgQUvYHpwQNyN0OT4Ey1SZjDceMBd+cwHwYDVR0jBBgwFoAUvYHpwQNy\n\
N0OT4Ey1SZjDceMBd+cwDwYDVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNHADBE\n\
AiAwjFxZKdpSB4Imo2kcMPcG5xpoKb98c46tFew6KG6ougIgJdTSxXnzpwH04VBn\n\
COHiU9k528QAlWHM6miimGi/m6U=\n\
-----END CERTIFICATE-----\n";

    // Each test owns its own directory, wiped on entry, so they stay independent
    // under the default parallel test runner and need no cleanup on the way out.
    fn temp_case_dir(case: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("skills_test_ca_bundles")
            .join(case);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_root_certs_default_to_bundled_roots() {
        let result = resolve_root_certs(None, None, None).unwrap();
        assert!(matches!(result, RootCerts::WebPki));
    }

    #[test]
    fn test_root_certs_strategy_selects_trust_store() {
        assert!(matches!(
            resolve_root_certs(Some("webpki"), None, None).unwrap(),
            RootCerts::WebPki
        ));
        assert!(matches!(
            resolve_root_certs(Some(" system "), None, None).unwrap(),
            RootCerts::PlatformVerifier
        ));
    }

    #[test]
    fn test_root_certs_rejects_strategy_combined_with_ca_bundle() {
        // Precedence cannot be enforced here: on unix the platform verifier
        // reads SSL_CERT_FILE/SSL_CERT_DIR instead of the OS store.
        let result = resolve_root_certs(
            Some("system"),
            Some(OsStr::new("/etc/ssl/cert.pem")),
            Some(OsStr::new("/etc/ssl/certs")),
        );

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::ConflictingTlsConfig { strategy, vars }
                if strategy == "system" && vars == ["SSL_CERT_FILE", "SSL_CERT_DIR"]
        ));
    }

    #[test]
    fn test_root_certs_rejects_bundled_roots_combined_with_ca_bundle() {
        let result =
            resolve_root_certs(Some("webpki"), Some(OsStr::new("/etc/ssl/cert.pem")), None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::ConflictingTlsConfig { vars, .. } if vars == ["SSL_CERT_FILE"]
        ));
    }

    #[test]
    fn test_root_certs_treats_empty_ca_variables_as_conflicting() {
        // On unix an empty-but-set SSL_CERT_FILE keeps rustls-native-certs from
        // reading the OS store, so `system` would end up with no roots at all.
        let result = resolve_root_certs(Some("system"), Some(OsStr::new("")), None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::ConflictingTlsConfig { vars, .. } if vars == ["SSL_CERT_FILE"]
        ));

        let dir_only = resolve_root_certs(Some("system"), None, Some(OsStr::new("")));

        assert!(matches!(
            dir_only.unwrap_err(),
            SkillsError::ConflictingTlsConfig { vars, .. } if vars == ["SSL_CERT_DIR"]
        ));
    }

    #[test]
    fn test_root_certs_loads_ca_bundle_directory() {
        let dir = temp_case_dir("dir_loads");
        fs::write(dir.join("test-ca.pem"), TEST_CA_PEM).unwrap();
        fs::write(dir.join("unrelated.txt"), "not a certificate\n").unwrap();
        fs::create_dir(dir.join("nested")).unwrap();

        let result = resolve_root_certs(None, None, Some(dir.as_os_str())).unwrap();

        assert!(matches!(result, RootCerts::Specific(certs) if certs.len() == 1));
    }

    #[test]
    fn test_root_certs_unions_ca_bundle_file_and_directory() {
        let dir = temp_case_dir("file_and_dir");
        let file = dir.join("bundle.pem");
        fs::write(&file, TEST_CA_PEM).unwrap();
        let certs_dir = dir.join("certs");
        fs::create_dir(&certs_dir).unwrap();
        fs::write(certs_dir.join("test-ca.pem"), TEST_CA_PEM).unwrap();

        let result =
            resolve_root_certs(None, Some(file.as_os_str()), Some(certs_dir.as_os_str())).unwrap();

        assert!(matches!(result, RootCerts::Specific(certs) if certs.len() == 2));
    }

    #[test]
    fn test_root_certs_reports_unreadable_ca_bundle_directory() {
        let result = resolve_root_certs(None, None, Some(OsStr::new("/nonexistent/certs")));

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidCaBundle { path, .. } if path == "/nonexistent/certs"
        ));
    }

    #[test]
    fn test_root_certs_rejects_ca_bundle_directory_without_certificates() {
        let dir = temp_case_dir("dir_empty");
        fs::write(dir.join("readme.txt"), "nothing here\n").unwrap();

        let result = resolve_root_certs(None, None, Some(dir.as_os_str()));

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidCaBundle { reason, .. } if reason.contains("no certificate found")
        ));
    }

    #[test]
    fn test_root_certs_rejects_unknown_strategy() {
        let result = resolve_root_certs(Some("webpkl"), None, None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidTlsRoots(value) if value == "webpkl"
        ));
    }

    #[test]
    fn test_root_certs_empty_env_values_fall_back_to_default() {
        let result =
            resolve_root_certs(Some("  "), Some(OsStr::new("")), Some(OsStr::new(""))).unwrap();
        assert!(matches!(result, RootCerts::WebPki));
    }

    #[test]
    fn test_root_certs_reports_unreadable_ca_bundle() {
        let result = resolve_root_certs(None, Some(OsStr::new("/nonexistent/bundle.pem")), None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidCaBundle { path, .. } if path == "/nonexistent/bundle.pem"
        ));
    }

    #[test]
    fn test_root_certs_loads_ca_bundle_file() {
        let path = temp_case_dir("file_loads").join("bundle.pem");
        fs::write(&path, TEST_CA_PEM).unwrap();

        let result = resolve_root_certs(None, Some(path.as_os_str()), None).unwrap();

        assert!(matches!(result, RootCerts::Specific(certs) if certs.len() == 1));
    }

    #[test]
    fn test_root_certs_rejects_ca_bundle_without_certificates() {
        let path = temp_case_dir("file_empty").join("empty.pem");
        fs::write(&path, "# nothing to see here\n").unwrap();

        let result = resolve_root_certs(None, Some(path.as_os_str()), None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidCaBundle { reason, .. } if reason.contains("no certificate found")
        ));
    }

    #[test]
    fn test_root_certs_rejects_ca_bundle_of_undecodable_certificates() {
        // Well-formed PEM framing wrapping base64 that is not certificate DER.
        // ureq would silently drop this and leave an empty root store.
        let path = temp_case_dir("file_garbage").join("garbage.pem");
        fs::write(
            &path,
            "-----BEGIN CERTIFICATE-----\ndGhpcyBpcyBub3QgYSBjZXJ0aWZpY2F0ZQ==\n-----END CERTIFICATE-----\n",
        )
        .unwrap();

        let result = resolve_root_certs(None, Some(path.as_os_str()), None);

        assert!(matches!(
            result.unwrap_err(),
            SkillsError::InvalidCaBundle { reason, .. } if reason.contains("not a usable trust anchor")
        ));
    }

    #[test]
    fn test_transport_error_classifies_certificate_failures() {
        let unknown_issuer = SkillsError::transport("io: invalid peer certificate: UnknownIssuer");
        assert!(matches!(unknown_issuer, SkillsError::TlsUnknownIssuer(_)));

        // Widening the trust store would not fix either of these.
        let expired =
            SkillsError::transport("io: invalid peer certificate: certificate expired: ...");
        assert!(matches!(expired, SkillsError::TlsCertInvalid(_)));

        let wrong_host = SkillsError::transport(
            "io: invalid peer certificate: certificate not valid for name \"example.com\"",
        );
        assert!(matches!(wrong_host, SkillsError::TlsCertInvalid(_)));

        let other = SkillsError::transport("io: connection refused");
        assert!(matches!(other, SkillsError::NetworkError(_)));
    }

    #[test]
    fn test_tarball_url() {
        let github_url = GitHubUrl {
            slug: "anthropics/skills".to_string(),
            r#ref: "main".to_string(),
            sha: "main".to_string(),
            path: "skills/frontend-design".to_string(),
        };

        assert_eq!(
            github_url.tarball_url(),
            "https://api.github.com/repos/anthropics/skills/tarball/main"
        );
    }

    #[test]
    fn test_commits_url_uses_sha_param() {
        let github_url = GitHubUrl {
            slug: "owner/repo".to_string(),
            r#ref: "feature/foo".to_string(),
            sha: "resolved-sha".to_string(),
            path: "skills/my skill".to_string(),
        };

        assert_eq!(
            github_url.commits_url(),
            "https://api.github.com/repos/owner/repo/commits?sha=feature%2Ffoo&path=skills%2Fmy%20skill&per_page=1"
        );
    }

    #[test]
    fn test_contents_url_encodes_path_segments() {
        let github_url = GitHubUrl {
            slug: "owner/repo".to_string(),
            r#ref: "release/v1.0".to_string(),
            sha: "abc123".to_string(),
            path: "skills/my skill".to_string(),
        };

        assert_eq!(
            github_url.contents_url(),
            "https://api.github.com/repos/owner/repo/contents/skills/my%20skill?ref=abc123"
        );
    }

    #[test]
    fn test_child_url_uses_parent_sha() {
        let github_url = GitHubUrl {
            slug: "owner/repo".to_string(),
            r#ref: "release/v1.0".to_string(),
            sha: "resolved-parent-sha".to_string(),
            path: "skills".to_string(),
        };
        let child = github_url.child("frontend-design");

        assert_eq!(child.slug, "owner/repo");
        assert_eq!(child.r#ref, "resolved-parent-sha");
        assert_eq!(child.sha, "resolved-parent-sha");
        assert_eq!(child.path, "skills/frontend-design");
    }
}
