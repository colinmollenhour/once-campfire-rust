//! The reference linker over what the message pipeline really produces.

use campfire_fork::References;
use campfire_richtext::dom::Dom;
use campfire_richtext::{AttachableResolver, GidLookup, RenderContext, SignedLookup, message_presentation};

struct NoRecords;

impl AttachableResolver for NoRecords {
    fn locate_signed(&self, _sgid: &str) -> SignedLookup {
        SignedLookup::Invalid
    }

    fn find_gid(&self, _gid: &str) -> GidLookup {
        GidLookup::NotFound
    }
}

fn presentation(body: &str) -> String {
    let ctx = RenderContext { resolver: &NoRecords, request_host: Some("once.campfire.test".into()) };
    message_presentation(body, &ctx).unwrap()
}

/// How many links a browser would parse out of `html`.
fn parsed_links(html: &str) -> usize {
    let mut dom = Dom::new();
    let root = dom.parse_fragment(html).unwrap();
    dom.descendants(root).into_iter().filter(|&node| dom.local_name(node) == Some("a")).count()
}

/// References in text become links, and ones in an autolinked URL, code, a code block or an
/// attribute stay as they are.
#[test]
fn links_references_only_in_text() {
    let references = References::from_json(
        r#"[
            { "gitlab": "https://gitlab.example.com", "group": "shipstream", "project": "server" },
            { "prefixes": ["DEV"], "url": "https://app.clickup.com/t/1/{id}" }
        ]"#,
    )
    .unwrap();
    let body = concat!(
        "<p>Fixed DEV-3400 in !2989, see https://app.clickup.com/t/1/DEV-3400 &amp; <code>!1</code></p>",
        "<pre>DEV-1</pre><p title=\"DEV-2\">&lt;b&gt;DEV-5&lt;/b&gt;</p>"
    );
    let linked = references.link(&presentation(body));
    let links: Vec<&str> = linked.split(" href=\"").skip(1).map(|rest| &rest[..rest.find('"').unwrap()]).collect();
    assert_eq!(
        links,
        [
            "https://app.clickup.com/t/1/DEV-3400",
            "https://gitlab.example.com/shipstream/server/-/merge_requests/2989",
            "https://app.clickup.com/t/1/DEV-3400",
            "https://app.clickup.com/t/1/DEV-5",
        ],
        "{linked}"
    );
    assert!(linked.contains("<code>!1</code>") && linked.contains("<pre>DEV-1</pre>") && linked.contains("title=\"DEV-2\""), "{linked}");
    assert_eq!(parsed_links(&linked), 4, "{linked}");
}
