//! Google Tasks API access. Nothing in here touches the UI or the database —
//! it turns HTTP into domain types and classified errors, and stops there.

pub mod client;
pub mod errors;
pub mod models;
