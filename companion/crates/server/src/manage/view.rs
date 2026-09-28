//! The management surface's markup, as pure functions over plain data.
//!
//! Rendering is where an injection bug would live, so it is kept free of Axum,
//! `ServerState` and IO: every function here is `&Model -> String` and testable
//! without a server. There is no template engine on purpose -- four pages of
//! forms do not justify a dependency -- and no external stylesheet or font,
//! because this page must render on a homelab with no egress to a CDN.

use std::fmt::Write as _;

use crate::oauth::IntegrationHealth;

/// Escapes text for interpolation into element content or a quoted attribute.
///
/// `&` must be replaced FIRST: doing it after the others would re-escape the
/// ampersands in the entities they just produced, so `<` would render as
/// `&amp;lt;` instead of `<`.
pub(crate) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) struct DeviceRow {
    pub(crate) id: String,
    pub(crate) connected: bool,
}

pub(crate) struct IntegrationRow {
    pub(crate) id: String,
    pub(crate) health: Option<IntegrationHealth>,
    pub(crate) has_producer_credential: bool,
}

pub(crate) struct SourceRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) has_frame: bool,
    pub(crate) stale: bool,
    pub(crate) last_push: Option<String>,
}

pub(crate) struct DashboardModel {
    pub(crate) devices: Vec<DeviceRow>,
    pub(crate) integrations: Vec<IntegrationRow>,
    pub(crate) sources: Vec<SourceRow>,
}

const STYLE: &str = "\
body{font-family:system-ui,sans-serif;margin:2rem auto;max-width:52rem;padding:0 1rem;\
background:#111;color:#eee}\
h1{font-size:1.4rem}h2{font-size:1.05rem;margin-top:2rem;border-bottom:1px solid #333;\
padding-bottom:.3rem}\
table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:.4rem .6rem;\
border-bottom:1px solid #262626;vertical-align:middle}\
th{color:#999;font-weight:500;font-size:.85rem}\
.ok{color:#7ddc8a}.warn{color:#e8c76a}.bad{color:#e88c8c}.muted{color:#777}\
button{font:inherit;padding:.25rem .6rem;background:#222;color:#eee;border:1px solid #444;\
border-radius:4px;cursor:pointer}button:hover{background:#2c2c2c}\
form{display:inline}\
code{background:#1c1c1c;padding:.15rem .35rem;border-radius:3px;word-break:break-all}\
.token{display:block;padding:.8rem;margin:1rem 0;background:#1c1c1c;border:1px solid #444;\
border-radius:4px}";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{}</title><style>{STYLE}</style></head><body>{body}</body></html>",
        escape_html(title)
    )
}

pub(crate) fn render_dashboard(model: &DashboardModel) -> String {
    let mut body = String::from("<h1>Deskmate</h1>");

    body.push_str("<h2>Devices</h2>");
    if model.devices.is_empty() {
        body.push_str("<p class=\"muted\">No devices minted.</p>");
    } else {
        body.push_str("<table><tr><th>Device</th><th>Link</th></tr>");
        for device in &model.devices {
            let (class, label) = if device.connected {
                ("ok", "Connected")
            } else {
                // A board at rest is the resting state, not a fault.
                ("muted", "Not connected")
            };
            let _ = write!(
                body,
                "<tr><td><code>{}</code></td><td class=\"{class}\">{label}</td></tr>",
                escape_html(&device.id)
            );
        }
        body.push_str("</table>");
    }

    body.push_str("<h2>Integrations</h2>");
    if model.integrations.is_empty() {
        body.push_str("<p class=\"muted\">No integrations connected.</p>");
    } else {
        body.push_str(
            "<table><tr><th>Integration</th><th>Health</th><th>Producer</th><th></th></tr>",
        );
        for integration in &model.integrations {
            let class = match integration.health.as_ref() {
                Some(IntegrationHealth::Connected) => "ok",
                Some(IntegrationHealth::NeedsReconnect) => "bad",
                Some(IntegrationHealth::Error(_)) | None => "warn",
            };
            let health = integration
                .health
                .as_ref()
                .map_or_else(|| "Unknown".to_string(), |health| format!("{health:?}"));
            let id = escape_html(&integration.id);
            let producer = if integration.has_producer_credential {
                "<span class=\"ok\">issued</span>"
            } else {
                "<span class=\"muted\">none</span>"
            };
            let _ = write!(
                body,
                "<tr><td><code>{id}</code></td><td class=\"{class}\">{}</td><td>{producer}</td><td>\
<form method=\"post\" action=\"/v1/manage/integrations/{id}/connect\"><button>Reconnect</button></form> \
<form method=\"post\" action=\"/v1/manage/integrations/{id}/producer\"><button>New producer credential</button></form> \
<form method=\"post\" action=\"/v1/manage/integrations/{id}/revoke\"><button>Revoke</button></form>\
</td></tr>",
                escape_html(&health)
            );
        }
        body.push_str("</table>");
    }
    body.push_str(
        "<form method=\"post\" action=\"/v1/manage/integrations/google/connect\">\
<button>Connect Google</button></form>",
    );

    body.push_str("<h2>Picture sources</h2>");
    if model.sources.is_empty() {
        body.push_str("<p class=\"muted\">No picture sources.</p>");
    } else {
        body.push_str(
            "<table><tr><th>Source</th><th>Name</th><th>State</th><th>Last push</th></tr>",
        );
        for source in &model.sources {
            let (class, label) = if !source.has_frame {
                ("muted", "Never pushed")
            } else if source.stale {
                ("warn", "Stale")
            } else {
                ("ok", "Fresh")
            };
            let _ = write!(
                body,
                "<tr><td><code>{}</code></td><td>{}</td><td class=\"{class}\">{label}</td><td class=\"muted\">{}</td></tr>",
                escape_html(&source.id),
                escape_html(&source.name),
                escape_html(source.last_push.as_deref().unwrap_or("--")),
            );
        }
        body.push_str("</table>");
    }

    // The two axes read together, which is this page's whole justification: a
    // stale source with a healthy integration means the producer is broken; a
    // stale source with NeedsReconnect means the owner must re-authorize.
    body.push_str(
        "<p class=\"muted\">A stale source with a healthy integration means the producer \
stopped pushing. A stale source with an integration needing reconnection means the \
authorization lapsed, and only reconnecting fixes it.</p>",
    );

    page("Deskmate", &body)
}

/// A plain error page. Sign-in happens in the web app now; a failure here that
/// has nothing to do with the session must say what it is, not send the owner
/// to re-authenticate.
pub(crate) fn render_error(message: &str) -> String {
    let body = format!(
        "<h1>Deskmate</h1><p class=\"bad\">{}</p><p><a href=\"/v1/manage\">Back</a></p>",
        escape_html(message)
    );
    page("Deskmate", &body)
}

pub(crate) fn render_minted_credential(integration_id: &str, token: &str) -> String {
    let body = format!(
        "<h1>Producer credential</h1>\
<p>For integration <code>{}</code>. This is shown <strong>once</strong>; the server keeps \
only its SHA-256 digest and there is no way to read it back.</p>\
<code class=\"token\">{}</code>\
<p class=\"muted\">It vends access tokens for this integration only, and is revoked \
separately from the source token a producer pushes pictures with.</p>\
<p><a href=\"/v1/manage\">Back</a></p>",
        escape_html(integration_id),
        escape_html(token)
    );
    page("Deskmate — producer credential", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_every_character_that_can_leave_an_attribute_or_element() {
        assert_eq!(
            escape_html(r#"<script>"x"&'y'</script>"#),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn ampersand_is_escaped_first_so_entities_are_not_double_escaped() {
        // Replacing '&' last would turn the '&' of "&lt;" into "&amp;lt;".
        assert_eq!(escape_html("<"), "&lt;");
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
    }

    fn model_with_source(name: &str) -> DashboardModel {
        DashboardModel {
            devices: Vec::new(),
            integrations: Vec::new(),
            sources: vec![SourceRow {
                id: "image-1".to_string(),
                name: name.to_string(),
                has_frame: true,
                stale: false,
                last_push: Some("2026-09-13T00:00:00Z".to_string()),
            }],
        }
    }

    #[test]
    fn a_hostile_image_source_name_cannot_inject_markup() {
        // Source names are operator-supplied at mint time and pass through no
        // charset validation, unlike integration ids.
        let html = render_dashboard(&model_with_source("<img src=x onerror=alert(1)>"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img src=x"));
    }

    #[test]
    fn the_two_health_axes_are_rendered_distinctly() {
        let model = DashboardModel {
            devices: vec![DeviceRow {
                id: "dev-0005".to_string(),
                connected: false,
            }],
            integrations: vec![IntegrationRow {
                id: "google".to_string(),
                health: Some(IntegrationHealth::NeedsReconnect),
                has_producer_credential: true,
            }],
            sources: vec![SourceRow {
                id: "image-1".to_string(),
                name: "calendar".to_string(),
                has_frame: true,
                stale: true,
                last_push: Some("2026-09-13T00:00:00Z".to_string()),
            }],
        };
        let html = render_dashboard(&model);
        assert!(html.contains("Reconnect"));
        assert!(html.contains("Stale"));
        assert!(html.contains("NeedsReconnect"));
        // A disconnected board is the resting state, not an error.
        assert!(html.contains("Not connected"));
    }

    #[test]
    fn integration_health_classes_follow_variants_and_escape_details() {
        for (health, expected_cell) in [
            (None, "<td class=\"warn\">Unknown</td>"),
            (
                Some(IntegrationHealth::Connected),
                "<td class=\"ok\">Connected</td>",
            ),
            (
                Some(IntegrationHealth::NeedsReconnect),
                "<td class=\"bad\">NeedsReconnect</td>",
            ),
            (
                Some(IntegrationHealth::Error(
                    "NeedsReconnect <retry & wait>".to_string(),
                )),
                "<td class=\"warn\">Error(&quot;NeedsReconnect &lt;retry &amp; wait&gt;&quot;)</td>",
            ),
        ] {
            let html = render_dashboard(&DashboardModel {
                devices: Vec::new(),
                integrations: vec![IntegrationRow {
                    id: "google".to_string(),
                    health,
                    has_producer_credential: false,
                }],
                sources: Vec::new(),
            });
            assert!(
                html.contains(expected_cell),
                "missing {expected_cell}: {html}"
            );
        }
    }

    #[test]
    fn a_source_that_was_never_pushed_is_not_reported_as_stale() {
        let mut model = model_with_source("kitchen");
        model.sources[0].has_frame = false;
        model.sources[0].stale = false;
        model.sources[0].last_push = None;
        let html = render_dashboard(&model);
        assert!(html.contains("Never pushed"));
        assert!(!html.contains("Stale"));
    }

    #[test]
    fn an_error_page_is_not_a_sign_in_form() {
        // Rendering the login form for a non-auth failure would tell the
        // operator to re-authenticate, which cannot fix it.
        let html = render_error("Too many consent flows are already pending.");
        assert!(html.contains("Too many consent flows"));
        assert!(!html.contains("<form"));
        assert!(!html.contains("Admin token"));
    }

    #[test]
    fn the_minted_credential_page_says_it_is_shown_once() {
        let html = render_minted_credential("google", "abc123");
        assert!(html.contains("abc123"));
        assert!(html.to_lowercase().contains("once"));
    }

    #[test]
    fn an_empty_dashboard_still_renders_a_page() {
        let html = render_dashboard(&DashboardModel {
            devices: Vec::new(),
            integrations: Vec::new(),
            sources: Vec::new(),
        });
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("No devices minted."));
    }
}
