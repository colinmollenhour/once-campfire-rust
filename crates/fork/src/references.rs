//! Links references to other tools in a message's text, as the install configures them
//! (`CAMPFIRE_REFERENCES`): GitLab merge requests (`!2989`, `supersaas!12`, `group/project!12`) and
//! ids with known prefixes (`DEV-3400` for ClickUp, `SERVER-537` for Sentry). Not in the Rails
//! app (see FORK.md).
//!
//! It runs on `message_presentation`'s output, so stored bodies, plain text (search, push, bot
//! webhooks) and the editor keep what was typed. It only links text: not inside links, code,
//! code blocks, mentions or attachments. The output it works on is serialized with `<` and `>`
//! escaped everywhere but in tags (see `autolink`), so the text between tags is plain.

use std::collections::BTreeMap;

use regex::Regex;
use serde_json::Value;

/// The rules, in the order the configuration lists them.
#[derive(Clone, Debug, Default)]
pub struct References {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
enum Rule {
    /// `!123` in the default project, `name!123` in the group, `group/project!123` anywhere.
    GitLab { url: String, group: Option<String>, project: Option<String>, pattern: Regex },
    /// `PREFIX-ID`, where the ID is uppercase letters and digits.
    Ids { url: String, pattern: Regex },
}

/// Elements whose text is left alone, besides `span.mention`.
const SKIPPED: &[&str] = &["a", "code", "pre", "action-text-attachment", "figure", "figcaption"];

/// Elements without an end tag.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

impl References {
    /// Parses `CAMPFIRE_REFERENCES`: a JSON array of rules, each either
    /// `{"gitlab": URL, "group": PATH, "project": NAME}` (`group` and `project` optional) or
    /// `{"prefixes": [PREFIX, ...], "url": URL with {id}}`.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| format!("isn't JSON: {e}"))?;
        let Some(items) = value.as_array() else { return Err("should be a JSON array of rules".into()) };
        let rules = items.iter().enumerate().map(|(i, item)| Rule::parse(item).map_err(|e| format!("rule {}: {e}", i + 1)));
        Ok(Self { rules: rules.collect::<Result<_, _>>()? })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// `html` with its references linked.
    pub fn link(&self, html: &str) -> String {
        if self.rules.is_empty() {
            return html.to_string();
        }
        let mut out = String::with_capacity(html.len());
        let mut open: Vec<(String, bool)> = Vec::new();
        let mut rest = html;
        while !rest.is_empty() {
            let (text, tag, after) = match rest.find('<') {
                Some(at) => {
                    let end = tag_end(rest, at);
                    (&rest[..at], &rest[at..end], &rest[end..])
                }
                None => (rest, "", ""),
            };
            if open.iter().any(|(_, skipped)| *skipped) {
                out.push_str(text);
            } else {
                self.link_text(text, &mut out);
            }
            track(tag, &mut open);
            out.push_str(tag);
            rest = after;
        }
        out
    }

    fn link_text(&self, text: &str, out: &mut String) {
        let mut found: Vec<(usize, usize, String)> = Vec::new();
        let mut occupied = BTreeMap::<usize, usize>::new();
        for rule in &self.rules {
            for (start, end, url) in rule.find(text) {
                let overlaps_previous = occupied.range(..=start).next_back().is_some_and(|(_, previous_end)| *previous_end > start);
                let overlaps_next = occupied.range(start..).next().is_some_and(|(next_start, _)| *next_start < end);
                if !overlaps_previous && !overlaps_next {
                    occupied.insert(start, end);
                    found.push((start, end, url));
                }
            }
        }
        found.sort_by_key(|&(start, _, _)| start);
        let mut at = 0;
        for (start, end, url) in found {
            out.push_str(&text[at..start]);
            out.push_str(&format!("<a href=\"{}\" target=\"_blank\">{}</a>", attribute(&url), &text[start..end]));
            at = end;
        }
        out.push_str(&text[at..]);
    }
}

impl Rule {
    fn parse(item: &Value) -> Result<Self, String> {
        let text = |key: &str| -> Result<Option<String>, String> {
            match &item[key] {
                Value::Null => Ok(None),
                Value::String(s) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
                _ => Err(format!("“{key}” should be text")),
            }
        };
        let Some(object) = item.as_object() else { return Err("should be an object".into()) };
        if let Some(url) = text("gitlab")?.map(|url| url.trim_end_matches('/').to_string()) {
            if let Some(unknown) = object.keys().find(|k| !matches!(k.as_str(), "gitlab" | "group" | "project")) {
                return Err(format!("unknown key “{unknown}”"));
            }
            web_url(&url)?;
            let (group, project) = (text("group")?, text("project")?);
            if project.is_some() && group.is_none() {
                return Err("“project” needs a “group”".into());
            }
            // An optional project path (segments that start and end with a letter or digit), then
            // `!` and the merge request's number.
            let segment = "[A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?";
            let pattern = Regex::new(&format!("(?:{segment}(?:/{segment})*)?![0-9]+")).unwrap();
            return Ok(Self::GitLab { url, group, project, pattern });
        }
        if let Some(unknown) = object.keys().find(|k| !matches!(k.as_str(), "prefixes" | "url")) {
            return Err(format!("unknown key “{unknown}”"));
        }
        let url = text("url")?.ok_or("needs “gitlab”, or “prefixes” and “url”")?;
        web_url(&url)?;
        if !url.contains("{id}") {
            return Err("“url” should say where the id goes, with {id}".into());
        }
        let prefixes: Vec<&str> = item["prefixes"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if prefixes.is_empty() {
            return Err("“prefixes” should list at least one prefix".into());
        }
        let mut alternatives = Vec::new();
        for prefix in prefixes {
            let valid = prefix.starts_with(|c: char| c.is_ascii_uppercase())
                && prefix.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
                && !prefix.ends_with('-');
            if !valid {
                return Err(format!("“{prefix}” should be uppercase letters, digits and '-', like DEV"));
            }
            alternatives.push(regex::escape(prefix));
        }
        // Longest first, so DEVICE-SERVICE-12 isn't taken for DEVICE-…
        alternatives.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let pattern = Regex::new(&format!("(?:{})-[A-Z0-9]+", alternatives.join("|"))).unwrap();
        Ok(Self::Ids { url, pattern })
    }

    /// Each reference in `text` and its URL, as byte ranges.
    fn find(&self, text: &str) -> Vec<(usize, usize, String)> {
        let pattern = match self {
            Self::GitLab { pattern, .. } | Self::Ids { pattern, .. } => pattern,
        };
        let mut found = Vec::new();
        for m in pattern.find_iter(text) {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            let joined = |c: Option<char>, extra: &str| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || extra.contains(c));
            if joined(before, "-/.!") || joined(after, "-") {
                continue;
            }
            let url = match self {
                Self::GitLab { url, group, project, .. } => {
                    let reference = m.as_str();
                    let (path, number) = reference.split_once('!').expect("the pattern has a !");
                    let project = match (path, group, project) {
                        ("", Some(group), Some(project)) => format!("{group}/{project}"),
                        ("", _, _) => continue,
                        (path, _, _) if path.contains('/') => path.to_string(),
                        (name, Some(group), _) => format!("{group}/{name}"),
                        (_, None, _) => continue,
                    };
                    format!("{url}/{project}/-/merge_requests/{number}")
                }
                Self::Ids { url, .. } => url.replace("{id}", m.as_str()),
            };
            found.push((m.start(), m.end(), url));
        }
        found
    }
}

fn web_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("http://") { Ok(()) } else { Err(format!("“{url}” should be an http(s) URL")) }
}

/// Where the tag starting at `at` ends: after its `>`, outside quotes.
fn tag_end(html: &str, at: usize) -> usize {
    let mut quote = None;
    for (i, c) in html[at..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '>') => return at + i + 1,
            _ => {}
        }
    }
    html.len()
}

/// Keeps the stack of open elements, and whether each one's text is skipped.
fn track(tag: &str, open: &mut Vec<(String, bool)>) {
    let Some(inner) = tag.strip_prefix('<') else { return };
    if let Some(closing) = inner.strip_prefix('/') {
        let name = tag_name(closing);
        if let Some(at) = open.iter().rposition(|(open_name, _)| *open_name == name) {
            open.truncate(at);
        }
        return;
    }
    let name = tag_name(inner);
    if name.is_empty() || name.starts_with('!') || VOID.contains(&name.as_str()) || tag.ends_with("/>") {
        return;
    }
    let mention = name == "span" && class_list(tag).any(|class| class == "mention");
    let skipped = SKIPPED.contains(&name.as_str()) || mention;
    open.push((name, skipped));
}

fn tag_name(inner: &str) -> String {
    inner.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '!').collect::<String>().to_ascii_lowercase()
}

/// The words of the tag's `class` attribute.
fn class_list(tag: &str) -> impl Iterator<Item = &str> {
    let value = tag.find("class=\"").map(|at| {
        let rest = &tag[at + 7..];
        &rest[..rest.find('"').unwrap_or(rest.len())]
    });
    value.unwrap_or_default().split_ascii_whitespace()
}

/// A URL for an attribute: the configuration's text and the reference, which is ASCII letters,
/// digits and `/._-!`, so only `&` and `"` need escaping.
fn attribute(url: &str) -> String {
    url.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipstream() -> References {
        References::from_json(
            r#"[
                { "gitlab": "https://gitlab.example.com/", "group": "shipstream", "project": "server" },
                { "prefixes": ["DEV", "OPS"], "url": "https://app.clickup.com/t/2304761/{id}" },
                { "prefixes": ["SERVER", "DEVICE-SERVICE"], "url": "https://acme.sentry.io/issues/{id}/" }
            ]"#,
        )
        .unwrap()
    }

    fn a(url: &str, text: &str) -> String {
        format!("<a href=\"{url}\" target=\"_blank\">{text}</a>")
    }

    #[test]
    fn links_merge_requests() {
        let links = shipstream();
        let mr = |path: &str, n: &str| format!("https://gitlab.example.com/{path}/-/merge_requests/{n}");
        assert_eq!(
            links.link("<p>See !2989 and supersaas!12, or acme/tools/cli!3.</p>"),
            format!(
                "<p>See {} and {}, or {}.</p>",
                a(&mr("shipstream/server", "2989"), "!2989"),
                a(&mr("shipstream/supersaas", "12"), "supersaas!12"),
                a(&mr("acme/tools/cli", "3"), "acme/tools/cli!3")
            )
        );
        assert_eq!(links.link("<p>(!7)</p>"), format!("<p>({})</p>", a(&mr("shipstream/server", "7"), "!7")));
        for text in ["wow!!1", "!12a", "a.!3", "!x"] {
            assert_eq!(links.link(text), text, "{text}");
        }
    }

    #[test]
    fn links_prefixed_ids() {
        let links = shipstream();
        assert_eq!(
            links.link("DEV-3400 broke on SERVER-537; DEVICE-SERVICE-1A too."),
            format!(
                "{} broke on {}; {} too.",
                a("https://app.clickup.com/t/2304761/DEV-3400", "DEV-3400"),
                a("https://acme.sentry.io/issues/SERVER-537/", "SERVER-537"),
                a("https://acme.sentry.io/issues/DEVICE-SERVICE-1A/", "DEVICE-SERVICE-1A")
            )
        );
        for text in ["dev-3400", "ADEV-1", "DEV-1-2", "DEV-", "DEVICE-APP-12", "x/DEV-1", "DEV-1a"] {
            assert_eq!(links.link(text), text, "{text}");
        }
    }

    #[test]
    fn earlier_rules_win_over_overlapping_references() {
        let gitlab = r#"{"gitlab":"https://gitlab.example.com","group":"acme"}"#;
        let ids = r#"{"prefixes":["DEV"],"url":"https://issues.example.com/{id}"}"#;
        let gitlab_first = References::from_json(&format!("[{gitlab},{ids}]")).unwrap();
        let ids_first = References::from_json(&format!("[{ids},{gitlab}]")).unwrap();
        assert_eq!(
            gitlab_first.link("DEV-1!2 DEV-3"),
            format!(
                "{} {}",
                a("https://gitlab.example.com/acme/DEV-1/-/merge_requests/2", "DEV-1!2"),
                a("https://issues.example.com/DEV-3", "DEV-3")
            )
        );
        assert_eq!(
            ids_first.link("DEV-1!2 DEV-3"),
            format!("{}!2 {}", a("https://issues.example.com/DEV-1", "DEV-1"), a("https://issues.example.com/DEV-3", "DEV-3"))
        );
    }

    #[test]
    fn leaves_links_code_and_mentions_alone() {
        let links = shipstream();
        let html = concat!(
            "<div class=\"lexxy-content\">\n  <p><a href=\"https://x/DEV-1\">DEV-1</a> <code>!12</code></p>",
            "<pre>SERVER-2</pre><span class=\"mention\" sgid=\"s\"><a title=\"x\" class=\"btn avatar\" href=\"/users/1\">",
            "<img src=\"/a.png\" width=\"48\" height=\"48\" /></a> OPS-9 Team</span><br>OPS-9\n</div>\n"
        );
        let linked = links.link(html);
        // The link that was there, and OPS-9 after the mention.
        assert_eq!(linked.matches("<a href=").count(), 2, "{linked}");
        assert!(linked.ends_with(&format!("<br>{}\n</div>\n", a("https://app.clickup.com/t/2304761/OPS-9", "OPS-9"))));
        assert!(linked.contains("</a> OPS-9 Team</span>"), "{linked}");
    }

    #[test]
    fn keeps_text_without_references_as_it_is() {
        let html = "<p title=\"a &gt; b\">Fine &amp; dandy, no refs here! 100%</p>";
        assert_eq!(shipstream().link(html), html);
        assert_eq!(References::default().link("!12 DEV-1"), "!12 DEV-1");
    }

    #[test]
    fn refuses_bad_configuration() {
        let error = |json: &str| References::from_json(json).unwrap_err();
        assert!(error("{}").contains("JSON array"));
        assert!(error(r#"[{ "prefixes": ["dev"], "url": "https://x/{id}" }]"#).contains("uppercase"));
        assert!(error(r#"[{ "prefixes": ["DEV"], "url": "https://x/" }]"#).contains("{id}"));
        assert!(error(r#"[{ "prefixes": ["DEV"], "url": "javascript:{id}" }]"#).contains("http(s)"));
        assert!(error(r#"[{ "gitlab": "https://g", "project": "server" }]"#).contains("needs a “group”"));
        assert!(error(r#"[{ "gitlab": "https://g", "groups": "x" }]"#).contains("unknown key"));
        assert!(error(r#"[{ "url": "https://x/{id}" }]"#).starts_with("rule 1:"));
    }
}
