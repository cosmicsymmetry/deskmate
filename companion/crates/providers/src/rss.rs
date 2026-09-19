use protocol::truncate_utf8_to_bytes;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::http::{HttpClient, validate_http_url};
use crate::{LastGood, ProviderError, ProviderSnapshot};

const MAX_RSS_ITEMS: usize = 5;
const MAX_XML_DEPTH: usize = 32;
const MAX_XML_EVENTS: usize = 100_000;
const MAX_ITEM_TEXT_BYTES: usize = 4_096;
const ACTIVE_MARKERS: [&str; 23] = [
    "<script",
    "</script",
    "<style",
    "<iframe",
    "<object",
    "<embed",
    "<form",
    "<svg",
    "<math",
    "<img",
    "<audio",
    "<video",
    "<link",
    "<meta",
    "<base",
    "javascript:",
    "vbscript:",
    "data:text/html",
    "onerror",
    "onload",
    "onclick",
    "onfocus",
    "onmouseover",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssOptions {
    pub url: String,
    pub maximum_items: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedItem {
    pub title: String,
    pub link: Option<String>,
    pub published: Option<String>,
}

#[derive(Default)]
struct FeedItemAccumulator {
    item: FeedItem,
    updated: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RssFeed {
    pub items: Vec<FeedItem>,
}

pub struct RssProvider<C> {
    client: C,
    options: RssOptions,
    state: LastGood<RssFeed>,
}

impl<C: HttpClient> RssProvider<C> {
    pub fn new(client: C, options: RssOptions) -> Self {
        Self {
            client,
            options,
            state: LastGood::default(),
        }
    }

    pub fn refresh(&mut self) -> ProviderSnapshot<RssFeed> {
        let result = self
            .client
            .get_text(&self.options.url)
            .and_then(|body| parse_rss(&body, self.options.maximum_items.min(MAX_RSS_ITEMS)));
        self.state.complete(result)
    }
}

#[allow(clippy::too_many_lines)]
fn parse_rss(body: &str, maximum_items: usize) -> Result<RssFeed, ProviderError> {
    if maximum_items == 0 || maximum_items > MAX_RSS_ITEMS {
        return Err(ProviderError::InvalidConfiguration(
            "RSS item count is outside the supported range".into(),
        ));
    }
    let mut reader = Reader::from_str(body);
    let mut stack = Vec::<String>::new();
    let mut current = None::<FeedItemAccumulator>;
    let mut items = Vec::with_capacity(maximum_items);
    let mut saw_root = false;
    let mut events = 0_usize;

    loop {
        events += 1;
        if events > MAX_XML_EVENTS {
            return Err(ProviderError::MalformedFeed(
                "RSS XML has too many nodes".into(),
            ));
        }
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let name = local_name(start.name().as_ref())?;
                reject_active_element(&name)?;
                reject_active_attributes(&start)?;
                if stack.is_empty() {
                    saw_root = matches!(name.as_str(), "rss" | "feed" | "rdf");
                }
                stack.push(name.clone());
                if stack.len() > MAX_XML_DEPTH {
                    return Err(ProviderError::MalformedFeed(
                        "RSS XML is too deeply nested".into(),
                    ));
                }
                if matches!(name.as_str(), "item" | "entry") {
                    if current.is_some() {
                        return Err(ProviderError::MalformedFeed("nested RSS item".into()));
                    }
                    current = Some(FeedItemAccumulator::default());
                }
                if name == "link"
                    && current.is_some()
                    && let Some(href) = attribute(&start, b"href")?
                {
                    assign_link(&mut current.as_mut().expect("checked above").item, &href)?;
                }
            }
            Ok(Event::Empty(start)) => {
                let name = local_name(start.name().as_ref())?;
                reject_active_element(&name)?;
                reject_active_attributes(&start)?;
                if name == "link"
                    && let Some(item) = current.as_mut()
                    && let Some(href) = attribute(&start, b"href")?
                {
                    assign_link(&mut item.item, &href)?;
                }
            }
            Ok(Event::End(end)) => {
                let name = local_name(end.name().as_ref())?;
                let opened = stack.pop().ok_or_else(|| {
                    ProviderError::MalformedFeed("RSS XML closing tag is unmatched".into())
                })?;
                if opened != name {
                    return Err(ProviderError::MalformedFeed(
                        "RSS XML tags are not balanced".into(),
                    ));
                }
                if matches!(name.as_str(), "item" | "entry") {
                    let item = current.take().ok_or_else(|| {
                        ProviderError::MalformedFeed("RSS item boundary is invalid".into())
                    })?;
                    let mut feed_item = item.item;
                    feed_item.title = plain_text(&feed_item.title)?;
                    if feed_item.title.is_empty() {
                        feed_item.title = "(Untitled)".into();
                    }
                    if let Some(link) = feed_item.link.take() {
                        assign_link(&mut feed_item, &link)?;
                    }
                    let published = feed_item
                        .published
                        .as_deref()
                        .map(plain_text)
                        .transpose()?
                        .filter(|value| !value.is_empty());
                    let updated = item
                        .updated
                        .as_deref()
                        .map(plain_text)
                        .transpose()?
                        .filter(|value| !value.is_empty());
                    feed_item.published = published.or(updated);
                    if items.len() < maximum_items {
                        items.push(feed_item);
                    }
                }
            }
            Ok(Event::Text(text)) => {
                let decoded = text.decode().map_err(|_| ProviderError::InvalidEncoding)?;
                append_item_text(&stack, current.as_mut(), &decoded)?;
            }
            Ok(Event::CData(text)) => {
                let decoded = text.decode().map_err(|_| ProviderError::InvalidEncoding)?;
                append_item_text(&stack, current.as_mut(), &decoded)?;
            }
            Ok(Event::DocType(_) | Event::PI(_)) => return Err(ProviderError::UnsafeContent),
            Ok(Event::Eof) => break,
            Ok(Event::GeneralRef(reference)) => {
                let decoded = reference
                    .decode()
                    .map_err(|_| ProviderError::InvalidEncoding)?;
                let replacement = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|_| ProviderError::MalformedFeed("RSS entity is invalid".into()))?
                {
                    character.to_string()
                } else {
                    quick_xml::escape::resolve_xml_entity(&decoded)
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            ProviderError::MalformedFeed("RSS entity is invalid".into())
                        })?
                };
                append_item_text(&stack, current.as_mut(), &replacement)?;
            }
            Ok(Event::Decl(_) | Event::Comment(_)) => {}
            Err(_) => {
                return Err(ProviderError::MalformedFeed(
                    "RSS XML syntax is invalid".into(),
                ));
            }
        }
    }
    if !saw_root || !stack.is_empty() || current.is_some() {
        return Err(ProviderError::MalformedFeed(
            "RSS/Atom document boundary is invalid".into(),
        ));
    }
    Ok(RssFeed { items })
}

fn append_item_text(
    stack: &[String],
    current: Option<&mut FeedItemAccumulator>,
    text: &str,
) -> Result<(), ProviderError> {
    let Some(item) = current else {
        return Ok(());
    };
    let Some(element) = stack.last().map(String::as_str) else {
        return Ok(());
    };
    let date_bytes = item
        .item
        .published
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(item.updated.as_ref().map_or(0, String::len));
    let target = match element {
        "title" => &mut item.item.title,
        "link" => item.item.link.get_or_insert_with(String::new),
        "pubdate" | "published" => item.item.published.get_or_insert_with(String::new),
        "updated" => item.updated.get_or_insert_with(String::new),
        _ => return Ok(()),
    };
    let existing_bytes = if matches!(element, "pubdate" | "published" | "updated") {
        date_bytes
    } else {
        target.len()
    };
    if existing_bytes.saturating_add(text.len()) > MAX_ITEM_TEXT_BYTES {
        return Err(ProviderError::MalformedFeed(
            "RSS item text is too long".into(),
        ));
    }
    target.push_str(text);
    Ok(())
}

fn assign_link(item: &mut FeedItem, raw: &str) -> Result<(), ProviderError> {
    let link = plain_text(raw)?;
    if link.is_empty() {
        return Ok(());
    }
    validate_http_url(&link).map_err(|_| ProviderError::UnsafeContent)?;
    item.link = Some(truncate_utf8_to_bytes(&link, 2_048).to_owned());
    Ok(())
}

fn plain_text(raw: &str) -> Result<String, ProviderError> {
    let lowercase = raw.to_ascii_lowercase();
    if ACTIVE_MARKERS
        .iter()
        .any(|marker| lowercase.contains(marker))
    {
        return Err(ProviderError::UnsafeContent);
    }
    let mut output = String::with_capacity(raw.len().min(128));
    let mut inside_tag = false;
    for character in raw.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    if inside_tag {
        return Err(ProviderError::UnsafeContent);
    }
    let collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok(truncate_utf8_to_bytes(&collapsed, 128).to_owned())
}

fn reject_active_element(name: &str) -> Result<(), ProviderError> {
    if matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "form"
            | "svg"
            | "math"
            | "img"
            | "audio"
            | "video"
            | "meta"
            | "base"
    ) {
        Err(ProviderError::UnsafeContent)
    } else {
        Ok(())
    }
}

fn reject_active_attributes(start: &BytesStart<'_>) -> Result<(), ProviderError> {
    for attribute in start.attributes() {
        let attribute = attribute
            .map_err(|_| ProviderError::MalformedFeed("RSS attribute is invalid".into()))?;
        let local_name = attribute.key.local_name();
        let name = local_name.as_ref();
        if name.len() >= 2 && name[..2].eq_ignore_ascii_case(b"on") {
            return Err(ProviderError::UnsafeContent);
        }
    }
    Ok(())
}

fn local_name(raw: &[u8]) -> Result<String, ProviderError> {
    let raw = std::str::from_utf8(raw).map_err(|_| ProviderError::InvalidEncoding)?;
    Ok(raw
        .rsplit_once(':')
        .map_or(raw, |(_, local)| local)
        .to_ascii_lowercase())
}

fn attribute(start: &BytesStart<'_>, sought: &[u8]) -> Result<Option<String>, ProviderError> {
    for attribute in start.attributes() {
        let attribute = attribute
            .map_err(|_| ProviderError::MalformedFeed("RSS attribute is invalid".into()))?;
        if attribute
            .key
            .local_name()
            .as_ref()
            .eq_ignore_ascii_case(sought)
        {
            let value = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, start.decoder())
                .map_err(|_| ProviderError::MalformedFeed("RSS attribute is invalid".into()))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_and_atom_to_plain_bounded_rows() {
        let rss = parse_rss(include_str!("../tests/fixtures/feed.rss"), 5).unwrap();
        assert_eq!(rss.items.len(), 2);
        assert_eq!(rss.items[0].title, "Release 1.0 is ready");
        assert_eq!(rss.items[1].title, "Second & safe");
        assert_eq!(
            rss.items[0].link.as_deref(),
            Some("https://example.test/releases/1")
        );
        assert_eq!(
            rss.items[0].published.as_deref(),
            Some("Wed, 05 Aug 2026 10:00:00 GMT")
        );

        let atom = parse_rss(include_str!("../tests/fixtures/feed.atom"), 1).unwrap();
        assert_eq!(atom.items.len(), 1);
        assert_eq!(atom.items[0].title, "Atom headline");
        assert_eq!(
            atom.items[0].link.as_deref(),
            Some("https://example.test/atom/1")
        );
        assert_eq!(
            atom.items[0].published.as_deref(),
            Some("2026-08-05T10:00:00Z")
        );
    }

    #[test]
    fn atom_published_wins_over_updated_in_either_xml_order() {
        for date_elements in [
            "<published>2026-08-05T10:00:00Z</published><updated>2026-08-05T11:00:00Z</updated>",
            "<updated>2026-08-05T11:00:00Z</updated><published>2026-08-05T10:00:00Z</published>",
        ] {
            let feed = format!("<feed><entry><title>News</title>{date_elements}</entry></feed>");
            let parsed = parse_rss(&feed, 1).expect("the Atom entry parses");
            assert_eq!(
                parsed.items[0].published.as_deref(),
                Some("2026-08-05T10:00:00Z")
            );
        }
    }

    #[test]
    fn empty_published_falls_back_to_updated() {
        let feed = "<feed><entry><title>News</title><published> \n </published><updated>2026-08-05T11:00:00Z</updated></entry></feed>";
        let parsed = parse_rss(feed, 1).expect("the Atom entry parses");
        assert_eq!(
            parsed.items[0].published.as_deref(),
            Some("2026-08-05T11:00:00Z")
        );
    }

    #[test]
    fn date_text_accumulates_across_entity_and_cdata_events() {
        let feed = "<feed><entry><title>Entity</title><published>2026-08-05T10:&#48;0:00Z</published></entry><entry><title>CDATA</title><updated>2026-08-05T<![CDATA[11:00:00Z]]></updated></entry></feed>";
        let parsed = parse_rss(feed, 2).expect("the Atom entries parse");
        assert_eq!(
            parsed.items[0].published.as_deref(),
            Some("2026-08-05T10:00:00Z")
        );
        assert_eq!(
            parsed.items[1].published.as_deref(),
            Some("2026-08-05T11:00:00Z")
        );
    }

    #[test]
    fn unsafe_unselected_updated_text_is_rejected() {
        let feed = "<feed><entry><title>News</title><published>2026-08-05T10:00:00Z</published><updated>&lt;script&gt;bad()&lt;/script&gt;</updated></entry></feed>";
        assert_eq!(parse_rss(feed, 1), Err(ProviderError::UnsafeContent));
    }

    #[test]
    fn published_and_updated_share_the_item_text_ceiling() {
        let published = "p".repeat(MAX_ITEM_TEXT_BYTES / 2);
        let updated = "u".repeat(MAX_ITEM_TEXT_BYTES / 2 + 1);
        let feed = format!(
            "<feed><entry><title>News</title><published>{published}</published><updated>{updated}</updated></entry></feed>"
        );
        assert!(matches!(
            parse_rss(&feed, 1),
            Err(ProviderError::MalformedFeed(detail)) if detail == "RSS item text is too long"
        ));
    }

    #[test]
    fn strips_inert_markup_and_rejects_active_content_and_doctypes() {
        let harmless = r"<rss><channel><item><title>&lt;b&gt;Bold&lt;/b&gt; text</title></item></channel></rss>";
        assert_eq!(parse_rss(harmless, 1).unwrap().items[0].title, "Bold text");

        for unsafe_feed in [
            r"<rss><channel><item><title>&lt;script&gt;bad()&lt;/script&gt;</title></item></channel></rss>",
            r#"<!DOCTYPE rss [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><rss/>"#,
            r"<rss><channel><item><title>ok</title><link>javascript:alert(1)</link></item></channel></rss>",
            r#"<rss><channel><item><title onclick="bad()">ok</title></item></channel></rss>"#,
            r#"<rss><channel><item><title>ok<img src="https://example.test/pixel"/></title></item></channel></rss>"#,
        ] {
            assert!(matches!(
                &parse_rss(unsafe_feed, 1).unwrap_err(),
                ProviderError::UnsafeContent
            ));
        }
    }

    #[test]
    fn rejects_malformed_and_excessively_deep_xml() {
        assert!(parse_rss("<rss><item>", 1).is_err());
        let deep = format!(
            "<rss>{}<item><title>x</title></item>{}</rss>",
            "<x>".repeat(MAX_XML_DEPTH),
            "</x>".repeat(MAX_XML_DEPTH)
        );
        assert!(parse_rss(&deep, 1).is_err());
    }
}
