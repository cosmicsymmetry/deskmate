#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use server::ServerState;
use server::oauth::transport::{FetchResponse, OAuthFuture, OAuthTransport, TransportError};
use server::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager};
use server::secrets::{IntegrationSecret, IntegrationStore, SecretsKey};

pub struct IntegrationFixture {
    pub state: ServerState,
    _tempdir: tempfile::TempDir,
}

pub fn integration_state(
    transport: Arc<dyn OAuthTransport>,
    grant: Option<IntegrationSecret>,
) -> IntegrationFixture {
    let state = ServerState::in_memory();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(
        IntegrationStore::open(
            tempdir.path().join("secrets.enc"),
            SecretsKey::from_bytes([1u8; 32]),
        )
        .expect("store"),
    );
    if let Some(grant) = grant {
        store.put("google".to_string(), grant).expect("put");
    }
    let token_manager = Arc::new(TokenManager::new(
        store,
        transport,
        GoogleOAuthConfig::default(),
    ));
    state.set_integrations(Arc::new(IntegrationRuntime::new(token_manager)));
    IntegrationFixture {
        state,
        _tempdir: tempdir,
    }
}

/// Queued canned responses plus a record of every URL actually posted to, so a
/// test can assert the server reached exactly the hosts it should have.
pub struct FakeTransport {
    queued: Mutex<Vec<(u16, Vec<u8>)>>,
    urls: Mutex<Vec<String>>,
}

impl FakeTransport {
    pub fn with(responses: Vec<(u16, &str)>) -> Arc<Self> {
        Arc::new(Self {
            queued: Mutex::new(
                responses
                    .into_iter()
                    .rev()
                    .map(|(status, body)| (status, body.as_bytes().to_vec()))
                    .collect(),
            ),
            urls: Mutex::new(Vec::new()),
        })
    }

    pub fn hosts_called(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|url| url::Url::parse(url).ok())
            .filter_map(|url| url.host_str().map(str::to_owned))
            .collect()
    }
}

impl OAuthTransport for FakeTransport {
    fn post_form(
        &self,
        url: String,
        _form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
        self.urls.lock().unwrap().push(url);
        let next = self.queued.lock().unwrap().pop();
        Box::pin(async move {
            next.map(|(status, body)| FetchResponse { status, body })
                .ok_or_else(|| TransportError("no queued response".to_string()))
        })
    }
}
