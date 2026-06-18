use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObjectError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("ELF parse error in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: object::Error,
    },
    #[error("unsupported ELF class or architecture in {path}")]
    UnsupportedArch { path: String },
    #[error("unsupported archive format in {path}: {reason}")]
    UnsupportedArchive { path: String, reason: &'static str },
}

pub type Result<T> = std::result::Result<T, ObjectError>;
