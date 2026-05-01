#[cfg(any(feature = "imap", feature = "maildir"))]
pub mod get;
#[cfg(feature = "smtp")]
pub mod send;

pub mod command;
