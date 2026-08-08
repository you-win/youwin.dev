//! Markdown → sanitized HTML, plus a plaintext projection.
//!
//! Pipeline: parse → filter the event stream → render → sanitize. Raw HTML is
//! dropped twice on purpose (once at the parser, once at the sanitizer) because
//! `body_html` is the one value the templates interpolate without escaping.

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

/// Both derived columns, produced together so they can never disagree about
/// what the source said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Sanitized HTML. Safe to interpolate unescaped — and nothing else is.
    pub html: String,
    /// Plaintext, for OG descriptions and (at M5) the FTS index.
    pub text: String,
}

pub fn render(source: &str) -> Rendered {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let events: Vec<Event<'_>> = Parser::new_ext(source, options)
        // Raw HTML never reaches the renderer. The sanitizer would catch it
        // too, but dropping it here means the plaintext projection below is
        // also free of markup.
        .filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)))
        // The one non-default behaviour that matters. CommonMark collapses a
        // single newline into a space, which is wrong for a microblog: people
        // expect the line breaks they typed.
        .map(|event| match event {
            Event::SoftBreak => Event::HardBreak,
            other => other,
        })
        .collect();

    let events = autolink(events);

    let mut unsafe_html = String::with_capacity(source.len() + source.len() / 2);
    pulldown_cmark::html::push_html(&mut unsafe_html, events.iter().cloned());

    Rendered {
        html: sanitize(&unsafe_html),
        text: plain_text(&events),
    }
}

/// Allowlist sanitizer.
///
/// Built per call rather than cached: this runs at write time only — never on a
/// read — so the allocation is irrelevant next to the clarity of not reasoning
/// about shared mutable state.
fn sanitize(html: &str) -> String {
    let tags = HashSet::from([
        "p",
        "br",
        "em",
        "strong",
        "del",
        "s",
        "code",
        "pre",
        "a",
        "blockquote",
        "ul",
        "ol",
        "li",
        "hr",
    ]);

    // Headings survive as their text content (ammonia unwraps a disallowed tag
    // rather than dropping its children) — a 300-character post has no sections.
    // <img> has no children, so it vanishes entirely; there is no upload path in
    // v1, and allowing it later means adding one entry here.
    let mut builder = ammonia::Builder::default();
    builder
        .tags(tags)
        .tag_attributes(HashMap::from([("a", HashSet::from(["href"]))]))
        .generic_attributes(HashSet::new())
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .link_rel(Some("nofollow noopener noreferrer"))
        // A relative href on a microblog is always a mistake, and on the public
        // site it would resolve against youwin.dev and look deliberate.
        .url_relative(ammonia::UrlRelative::Deny);

    builder.clean(html).to_string()
}

/// Rewrites bare `http(s)://…` runs inside text into real links.
///
/// CommonMark only autolinks `<bracketed>` URLs, but pasting a raw URL is the
/// single most common thing anyone does in a microblog post.
fn autolink(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let mut out = Vec::with_capacity(events.len());
    // Text inside an existing link is already linked; text inside a fenced block
    // is code and must stay literal. (Inline code arrives as Event::Code, which
    // this loop never inspects, so it is safe by construction.)
    let mut in_link = false;
    let mut in_code_block = false;

    for event in events {
        match event {
            Event::Start(Tag::Link { .. }) => {
                in_link = true;
                out.push(event);
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
                out.push(event);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                out.push(event);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push(event);
            }
            Event::Text(ref text) if !in_link && !in_code_block => {
                push_autolinked(text, &mut out);
            }
            other => out.push(other),
        }
    }

    out
}

fn push_autolinked<'a>(text: &str, out: &mut Vec<Event<'a>>) {
    let mut cursor = 0;

    while let Some((start, end)) = find_url(text, cursor) {
        if start > cursor {
            out.push(Event::Text(text[cursor..start].to_owned().into()));
        }

        let url = &text[start..end];
        out.push(Event::Start(Tag::Link {
            link_type: LinkType::Autolink,
            dest_url: url.to_owned().into(),
            title: CowStr::Borrowed(""),
            id: CowStr::Borrowed(""),
        }));
        out.push(Event::Text(url.to_owned().into()));
        out.push(Event::End(TagEnd::Link));

        cursor = end;
    }

    if cursor < text.len() {
        out.push(Event::Text(text[cursor..].to_owned().into()));
    }
}

/// Locates the next bare URL at or after `from`, returning its byte range.
fn find_url(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut search = from;

    while let Some(offset) = text[search..].find("http") {
        let start = search + offset;
        let tail = &text[start..];

        let scheme_len = if tail.starts_with("https://") {
            8
        } else if tail.starts_with("http://") {
            7
        } else {
            0
        };

        // Must begin a word, so "shttp://x" and "foohttp://x" are left alone.
        let at_boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());

        if scheme_len > 0 && at_boundary {
            let mut end = start + tail.find(char::is_whitespace).unwrap_or(tail.len());

            // Trailing punctuation almost always belongs to the sentence, not
            // the URL. A closing paren is kept only when the URL opened one, so
            // Wikipedia-style links survive.
            while end > start {
                let candidate = &text[start..end];
                let Some(last) = candidate.chars().next_back() else {
                    break;
                };
                let unbalanced_paren =
                    last == ')' && candidate.matches('(').count() < candidate.matches(')').count();

                if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'') || unbalanced_paren
                {
                    end -= last.len_utf8();
                } else {
                    break;
                }
            }

            // A bare scheme with no host is not a link.
            if end > start + scheme_len {
                return Some((start, end));
            }
        }

        search = start + 4;
        if search >= text.len() {
            break;
        }
    }

    None
}

/// Plaintext projection, taken from the event stream rather than by stripping
/// tags out of the rendered HTML — the events are already free of markup, so
/// there is nothing to get wrong.
fn plain_text(events: &[Event<'_>]) -> String {
    let mut out = String::new();

    for event in events {
        match event {
            Event::Text(text) | Event::Code(text) => out.push_str(text),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_),
            ) => out.push('\n'),
            _ => {}
        }
    }

    out.trim().to_owned()
}

/// First `max_chars` characters, cut at a word boundary, for OG descriptions.
pub fn summarize(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");

    if flat.chars().count() <= max_chars {
        return flat;
    }

    let cut: String = flat.chars().take(max_chars).collect();
    let trimmed = match cut.rsplit_once(' ') {
        Some((head, _)) if head.len() > max_chars / 2 => head,
        _ => cut.as_str(),
    };

    format!("{}…", trimmed.trim_end_matches([' ', ',', '.', ';', ':']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_newlines_become_hard_breaks() {
        // The whole reason the event stream is rewritten: CommonMark would
        // render this as one line joined by a space.
        let out = render("first line\nsecond line");
        // pulldown-cmark writes "<br>\n"; the newline is insignificant whitespace,
        // so assert on the tag rather than pinning the exact byte sequence.
        assert!(out.html.contains("<br>"), "{}", out.html);
        assert!(out.html.starts_with("<p>first line"), "{}", out.html);
        assert!(out.html.contains("second line</p>"), "{}", out.html);
        assert_eq!(out.text, "first line\nsecond line");
    }

    #[test]
    fn raw_html_is_dropped_not_escaped() {
        let out = render("hello <script>alert(1)</script> <b>bold</b>");
        assert!(!out.html.contains("script"), "got: {}", out.html);
        assert!(!out.html.contains("<b>"), "got: {}", out.html);
        assert!(out.html.contains("hello"));
    }

    #[test]
    fn links_are_marked_up_and_restricted() {
        let out = render("[click](https://example.com) and [no](javascript:alert(1))");
        assert!(out.html.contains(r#"href="https://example.com""#), "{}", out.html);
        assert!(out.html.contains("nofollow"), "{}", out.html);
        assert!(!out.html.contains("javascript"), "{}", out.html);
    }

    #[test]
    fn bare_urls_autolink_without_swallowing_punctuation() {
        let out = render("see https://example.com/a, or https://example.com/b.");
        assert!(out.html.contains(r#"href="https://example.com/a""#), "{}", out.html);
        assert!(out.html.contains(r#"href="https://example.com/b""#), "{}", out.html);
        // The comma and full stop belong to the sentence.
        assert!(!out.html.contains("/a,"), "{}", out.html);
        assert!(!out.html.contains("/b."), "{}", out.html);
    }

    #[test]
    fn urls_inside_code_and_existing_links_are_left_alone() {
        let fenced = render("```\nhttps://example.com\n```");
        assert!(!fenced.html.contains("<a "), "{}", fenced.html);

        let inline = render("`https://example.com`");
        assert!(!inline.html.contains("<a "), "{}", inline.html);

        // Already a link: must not end up nested.
        let existing = render("[label](https://example.com)");
        assert_eq!(existing.html.matches("<a ").count(), 1, "{}", existing.html);
    }

    #[test]
    fn images_vanish_and_headings_degrade_to_text() {
        let image = render("![alt](https://example.com/x.png)");
        assert!(!image.html.contains("<img"), "{}", image.html);

        let heading = render("# Big\n\nbody");
        assert!(!heading.html.contains("<h1"), "{}", heading.html);
        assert!(heading.html.contains("Big"), "{}", heading.html);
    }

    #[test]
    fn allowed_formatting_survives() {
        let out = render("*a* **b** ~~c~~ `d`\n\n> quote\n\n- item");
        for expected in ["<em>a</em>", "<strong>b</strong>", "<del>c</del>", "<code>d</code>"] {
            assert!(out.html.contains(expected), "missing {expected} in {}", out.html);
        }
        assert!(out.html.contains("<blockquote>"), "{}", out.html);
        assert!(out.html.contains("<li>"), "{}", out.html);
    }

    #[test]
    fn summarize_cuts_on_a_word_boundary() {
        let text = "the quick brown fox jumps over the lazy dog and keeps running";
        let short = summarize(text, 20);
        assert!(short.ends_with('…'), "{short}");
        assert!(short.chars().count() <= 21, "{short}");
        assert!(!short.contains("  "));

        // Short input is returned whole, with no ellipsis.
        assert_eq!(summarize("short", 20), "short");
        // Newlines collapse, so a description never contains raw line breaks.
        assert_eq!(summarize("a\nb", 20), "a b");
    }
}
