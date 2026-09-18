//! Server-side OAuth integration framework. Provider-agnostic plumbing with
//! Google wired as the first identity; no `app-core`, config, or firmware
//! surface is touched.

mod pkce;
pub(crate) mod routes;
pub mod session;
mod token;
pub mod transport;

pub use routes::{IntegrationRuntime, PendingAuth};
pub use token::{GoogleOAuthConfig, TokenManager};
