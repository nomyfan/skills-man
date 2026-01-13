use std::fmt;
use std::io;

#[derive(Debug)]
pub enum SkillsError {
    // The GitHub URL did not match the expected pattern.
    InvalidUrl(String),

    // Any transport or connectivity failure when calling remote services.
    NetworkError(String),

    // The remote resource was not found (404).
    NotFound { url: String },

    // Access denied for the remote resource (403).
    Forbidden,

    // GitHub API rate limit was exceeded (429).
    RateLimited,

    // Non-OK HTTP status from the API with a message body.
    HttpError { status: u16, message: String },

    // The downloaded archive could not be parsed as gzip.
    InvalidArchive(String),

    // The requested path does not exist at the resolved ref.
    PathNotFound(String),

    // Skill directory missing the manifest file.
    MissingSkillManifest,

    // Local filesystem error surfaced during IO.
    IoError(io::Error),

    // skills.toml could not be parsed into the expected schema.
    ConfigParseError(String),
}

pub type SkillsResult<T> = Result<T, SkillsError>;

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
            SkillsError::NotFound { url } => write!(
                f,
                "Failed to access GitHub resource (HTTP 404)\n\nPossible reasons:\n  - Repository does not exist or has been deleted\n  - Branch/commit does not exist\n  - Repository is private\n\nURL: {url}"
            ),
            SkillsError::Forbidden => write!(
                f,
                "Access forbidden (HTTP 403)\n\nThe repository may be private. Private repositories are not currently supported."
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
            SkillsError::PathNotFound(path) => write!(
                f,
                "Path '{path}' not found in repository\n\nPossible reasons:\n  - Path is misspelled\n  - Path does not exist at the specified commit/branch"
            ),
            SkillsError::MissingSkillManifest => write!(
                f,
                "Invalid skill\n\nExpect 'SKILL.md' or 'skill.md' in the directory."
            ),
            SkillsError::IoError(err) => write!(f, "Filesystem error\n\n{err}"),
            SkillsError::ConfigParseError(reason) => write!(
                f,
                "Failed to parse skills.toml\n\nReason: {reason}\nPlease check the config file format."
            ),
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
