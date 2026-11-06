use std::fmt;
use std::io;

#[derive(Debug)]
pub enum SkillsError {
    // The GitHub URL did not match the expected pattern.
    InvalidUrl(String),

    // Any transport or connectivity failure when calling remote services.
    NetworkError(String),

    // The server certificate did not chain to any trusted root.
    TlsUnknownIssuer(String),

    // The server certificate was rejected for some other reason: expired, not
    // yet valid, wrong host, bad signature. Widening the trust store does not
    // help with any of those, so this one carries no such advice.
    TlsCertInvalid(String),

    // SKILLS_MAN_TLS_ROOTS holds a value that is not a known strategy.
    InvalidTlsRoots(String),

    // SKILLS_MAN_GITHUB_API_URL is not an http(s) URL.
    InvalidGitHubApiUrl(String),

    // Trust roots were configured twice, in ways that cannot both be honoured.
    ConflictingTlsConfig {
        strategy: String,
        vars: Vec<String>,
    },

    // SSL_CERT_FILE/SSL_CERT_DIR yielded no certificate usable as a trust anchor.
    InvalidCaBundle {
        path: String,
        reason: String,
    },

    // The remote resource was not found (404).
    NotFound {
        url: String,
    },

    // Access denied for the remote resource (403).
    Forbidden {
        url: String,
    },

    // GitHub API rate limit was exceeded (429).
    RateLimited,

    // Non-OK HTTP status from the API with a message body.
    HttpError {
        status: u16,
        message: String,
    },

    // The downloaded archive could not be parsed as gzip.
    InvalidArchive(String),

    // The requested paths do not exist at the resolved ref.
    PathNotFound(Vec<String>),

    // Skill directory missing the manifest file.
    MissingSkillManifest,

    // Local filesystem error surfaced during IO.
    IoError(io::Error),

    // skills.toml could not be parsed into the expected schema.
    ConfigParseError(String),

    // Directory contains neither a SKILL.md nor sub-skills
    NoSkillsFound(String),

    // One or more skills failed during batch installation
    BatchInstallationFailed {
        successful: usize,
        failed: Vec<String>,
    },

    // One or more skills dropped from a collection upstream could not be
    // removed locally
    BatchRemovalFailed {
        failed: Vec<String>,
    },

    // No registered provider can handle the given URL
    UnsupportedProvider(String),
}

pub type SkillsResult<T> = Result<T, SkillsError>;

impl SkillsError {
    // Classify a transport failure by its rendered message: rustls puts every
    // CertificateError behind one "invalid peer certificate" prefix, and only the
    // unknown-root case is worth answering with "widen the trust store".
    pub fn transport(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let lowered = reason.to_lowercase();
        if !lowered.contains("invalid peer certificate") {
            return SkillsError::NetworkError(reason);
        }
        if lowered.contains("unknownissuer") {
            SkillsError::TlsUnknownIssuer(reason)
        } else {
            SkillsError::TlsCertInvalid(reason)
        }
    }
}

impl fmt::Display for SkillsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkillsError::InvalidUrl(url) => write!(
                f,
                "Invalid GitHub URL\n\nExpected format: https://github.com/{{owner}}/{{repo}}/tree/{{ref}}/{{path}}\nGot: {url}"
            ),
            SkillsError::NetworkError(reason) => write!(
                f,
                "Network connection failed\n\nReason: {reason}\nPlease check your network connection and try again."
            ),
            SkillsError::TlsUnknownIssuer(reason) => write!(
                f,
                "TLS certificate verification failed\n\nReason: {reason}\n\nThe server certificate does not chain to any root that 'skill' currently trusts.\nThis usually means TLS is intercepted by a corporate or sandbox proxy whose CA\nlives in the operating system trust store, which 'skill' does not use by default\n(curl and git do, so they would succeed here).\n\nIf you trust that proxy, widen the trust store explicitly:\n  - SKILLS_MAN_TLS_ROOTS=system      Trust the operating system store, like curl/git\n  - SSL_CERT_FILE=/path/bundle.pem   Trust the CAs in this PEM bundle"
            ),
            SkillsError::TlsCertInvalid(reason) => write!(
                f,
                "TLS certificate verification failed\n\nReason: {reason}\n\nThe server presented a certificate that is not acceptable: it may be expired,\nnot valid yet, issued for a different host, or otherwise malformed. Changing\nwhich root certificates 'skill' trusts will not help.\n\nIf the host is reachable in a browser without warnings, check the system clock\nfirst, then whether something is rewriting the connection."
            ),
            SkillsError::InvalidTlsRoots(value) => write!(
                f,
                "Invalid SKILLS_MAN_TLS_ROOTS value: '{value}'\n\nSupported values:\n  - system   Trust the operating system store, like curl/git\n  - webpki   Trust the bundled Mozilla root certificates (default)\n\nTo trust a specific CA bundle instead, unset SKILLS_MAN_TLS_ROOTS and set\nSSL_CERT_FILE to a PEM file."
            ),
            SkillsError::InvalidGitHubApiUrl(value) => write!(
                f,
                "Invalid SKILLS_MAN_GITHUB_API_URL value: '{value}'\n\nExpected an http(s) URL that stands in for https://api.github.com, for example:\n  - https://proxy.example.com/api.github.com\n  - https://proxy.example.com/https://api.github.com"
            ),
            SkillsError::ConflictingTlsConfig { strategy, vars } => write!(
                f,
                "Conflicting TLS trust configuration\n\nSKILLS_MAN_TLS_ROOTS={strategy} selects a trust store, but {} also set:\n{}\n\nThese configure the same thing in different ways, and on unix the result would\ndepend on the operating system. Pick one:\n  - Keep SKILLS_MAN_TLS_ROOTS={strategy} and unset the variable(s) above\n  - Or unset SKILLS_MAN_TLS_ROOTS and let those variables supply the roots\n\nNote that an empty value still counts as set: on unix it stops the platform\nfrom reading its own trust store.",
                if vars.len() == 1 {
                    "this variable is"
                } else {
                    "these variables are"
                },
                vars.iter()
                    .map(|var| format!("  - {var}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            SkillsError::InvalidCaBundle { path, reason } => write!(
                f,
                "Failed to load CA certificates\n\nSource: {path}\nReason: {reason}\n\nSSL_CERT_FILE must name a readable PEM file, and SSL_CERT_DIR must name\ndirectories holding PEM files. Together they have to yield at least one\ncertificate usable as a trust anchor."
            ),
            SkillsError::NotFound { url } => write!(
                f,
                "Failed to access GitHub resource (HTTP 404)\n\nPossible reasons:\n  - Repository does not exist or has been deleted\n  - Branch/commit does not exist\n  - Repository is private\n\nURL: {url}"
            ),
            SkillsError::Forbidden { url } => write!(
                f,
                "Access forbidden (HTTP 403)\n\nThe repository may be private. Private repositories are not currently supported.\n\nURL: {}",
                url
            ),
            SkillsError::RateLimited => write!(
                f,
                "GitHub API rate limit exceeded (HTTP 429)\n\nPlease try again later or wait about 1 hour for the limit to reset."
            ),
            SkillsError::HttpError { status, message } => {
                write!(f, "HTTP error {status}: {message}")
            }
            SkillsError::InvalidArchive(reason) => {
                write!(f, "Downloaded file is not a valid gzip archive\n\n{reason}")
            }
            SkillsError::PathNotFound(paths) => {
                let label = if paths.len() == 1 { "Path" } else { "Paths" };
                write!(
                    f,
                    "{label} not found in repository:\n{}\n\nPossible reasons:\n  - Path is misspelled\n  - Path does not exist at the specified commit/branch",
                    paths
                        .iter()
                        .map(|path| format!("  - {path}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            SkillsError::MissingSkillManifest => write!(
                f,
                "Invalid skill\n\nExpect 'SKILL.md' or 'skill.md' in the directory."
            ),
            SkillsError::IoError(err) => write!(f, "Filesystem error\n\n{err}"),
            SkillsError::ConfigParseError(reason) => write!(
                f,
                "Failed to parse skills.toml\n\nReason: {reason}\nPlease check the config file format."
            ),
            SkillsError::NoSkillsFound(path) => write!(
                f,
                "No skills found at path '{path}'\n\nThe directory contains neither a SKILL.md file nor any subdirectories with SKILL.md files."
            ),
            SkillsError::BatchInstallationFailed { successful, failed } => {
                write!(
                    f,
                    "Batch installation completed with failures\n\nSuccessfully installed: {successful}\nFailed skills:\n"
                )?;
                for skill in failed {
                    writeln!(f, "  - {}", skill)?;
                }
                Ok(())
            }
            SkillsError::BatchRemovalFailed { failed } => {
                writeln!(
                    f,
                    "Failed to remove skills no longer present in the collection:"
                )?;
                for skill in failed {
                    writeln!(f, "  - {}", skill)?;
                }
                Ok(())
            }
            SkillsError::UnsupportedProvider(url) => {
                write!(f, "No provider available that supports the URL: {url}")
            }
        }
    }
}

impl std::error::Error for SkillsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SkillsError::IoError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for SkillsError {
    fn from(value: io::Error) -> Self {
        SkillsError::IoError(value)
    }
}

impl<T> From<SkillsError> for SkillsResult<T> {
    fn from(value: SkillsError) -> Self {
        Err(value)
    }
}
