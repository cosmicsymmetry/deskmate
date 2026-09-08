//! Server-side OAuth integration framework (spec §2, §3, §6). Provider-agnostic
//! plumbing with Google wired as the first identity; no `app-core`, config, or
//! firmware surface is touched.

pub mod pkce;
pub mod token;
pub mod transport;

pub use token::{GoogleOAuthConfig, IntegrationHealth, TokenError, TokenManager};
