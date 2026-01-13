use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillsError {
    #[error(
        "Invalid GitHub URL\n\nExpected format: https://github.com/{{owner}}/{{repo}}/tree/{{ref}}/{{path}}\nGot: {0}"
    )]
    InvalidUrl(String),

    #[error(
        "Network connection failed\n\nReason: {0}\nPlease check your network connection and try again."
    )]
    NetworkError(String),

    #[error(
        "Failed to access GitHub resource (HTTP 404)\n\nPossible reasons:\n  - Repository does not exist or has been deleted\n  - Branch/commit does not exist\n  - Repository is private\n\nURL: {url}"
    )]
    NotFound { url: String },

    #[error(
        "Access forbidden (HTTP 403)\n\nThe repository may be private. Private repositories are not currently supported."
    )]
    Forbidden,

    #[error(
        "GitHub API rate limit exceeded (HTTP 429)\n\nPlease try again later or wait about 1 hour for the limit to reset."
    )]
    RateLimited,

    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("Downloaded file is not a valid gzip archive\n\n{0}")]
    InvalidArchive(String),

    #[error(
        "Path '{0}' not found in repository\n\nPossible reasons:\n  - Path is misspelled\n  - Path does not exist at the specified commit/branch"
    )]
    PathNotFound(String),

    #[error("Invalid skill, expect 'SKILL.md' or 'skill.md' in the directory.")]
    MissingSkillManifest,

    #[error("Filesystem error\n\n{0}")]
    IoError(#[from] io::Error),

    #[error("Failed to parse skills.toml\n\nReason: {0}\nPlease check the config file format.")]
    ConfigParseError(String),
}

pub type SkillsResult<T> = Result<T, SkillsError>;

impl<T> From<SkillsError> for SkillsResult<T> {
    fn from(value: SkillsError) -> Self {
        Err(value)
    }
}
