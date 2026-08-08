//! The Atom document at `/feed.xml`.
//!
//! Hand-built rather than templated: maud escapes for HTML, and XML needs its
//! own rules (notably `'` and `"` inside attributes). Getting that wrong
//! produces a feed that parses in some readers and not others, which is a
//! miserable bug to chase.

use crate::{
    db::posts::FeedRow,
    public::view::time_fmt,
    render::markdown,
};

fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

pub fn render(origin: &str, rows: &[FeedRow]) -> String {
    // `updated` on the document is the newest entry's timestamp, so a reader
    // polling an unchanged feed sees an unchanged value.
    let updated = rows
        .first()
        .map(|row| time_fmt::rfc3339(row.post.created_at))
        .unwrap_or_else(|| time_fmt::rfc3339(0));

    let mut out = String::with_capacity(1024 + rows.len() * 512);

    out.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    out.push('\n');
    out.push_str(r#"<feed xmlns="http://www.w3.org/2005/Atom">"#);
    out.push('\n');

    out.push_str(&format!("  <title>youwin.dev</title>\n"));
    out.push_str(&format!("  <subtitle>Notes, mostly about software.</subtitle>\n"));
    out.push_str(&format!("  <id>{}/</id>\n", escape(origin)));
    out.push_str(&format!("  <updated>{updated}</updated>\n"));
    out.push_str(&format!(
        "  <link rel=\"alternate\" type=\"text/html\" href=\"{}/\"/>\n",
        escape(origin)
    ));
    out.push_str(&format!(
        "  <link rel=\"self\" type=\"application/atom+xml\" href=\"{}/feed.xml\"/>\n",
        escape(origin)
    ));
    out.push_str("  <author><name>youwin</name></author>\n");

    for row in rows {
        let url = format!("{origin}/p/{}", row.post.public_id);
        let published = time_fmt::rfc3339(row.post.created_at);
        // An edit should move the entry for subscribers; publication date should not.
        let updated = row
            .post
            .edited_at
            .map_or_else(|| published.clone(), time_fmt::rfc3339);

        // Microblog posts have no titles, so the first line of the body stands in.
        // Readers that render titles get something meaningful; readers that show
        // full content are unaffected.
        let title = markdown::summarize(&row.post.body_text, 80);

        out.push_str("  <entry>\n");
        out.push_str(&format!("    <title>{}</title>\n", escape(&title)));
        out.push_str(&format!("    <id>{}</id>\n", escape(&url)));
        out.push_str(&format!(
            "    <link rel=\"alternate\" type=\"text/html\" href=\"{}\"/>\n",
            escape(&url)
        ));
        out.push_str(&format!("    <published>{published}</published>\n"));
        out.push_str(&format!("    <updated>{updated}</updated>\n"));
        // The body is already sanitized HTML; escaped here so it travels as a
        // text node rather than as markup inside the XML.
        out.push_str(&format!(
            "    <content type=\"html\">{}</content>\n",
            escape(&row.post.body_html)
        ));
        out.push_str("  </entry>\n");
    }

    out.push_str("</feed>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::posts::{Post, Visibility};

    fn row(public_id: &str, body_html: &str, body_text: &str) -> FeedRow {
        FeedRow {
            post: Post {
                id: 1,
                public_id: public_id.to_owned(),
                parent_id: None,
                root_id: 1,
                body_html: body_html.to_owned(),
                body_text: body_text.to_owned(),
                visibility: Visibility::Public,
                // The Atom document has no notion of mood, which is the point of
                // asserting it here: nothing in this file may start rendering it.
                mood: None,
                created_at: 1_786_259_199_000,
                edited_at: None,
            },
            reply_count: 0,
        }
    }

    #[test]
    fn entities_are_escaped_including_quotes() {
        let out = render(
            "https://youwin.dev",
            &[row("abc", r#"<p>a &amp; b "c" 'd'</p>"#, "a & b")],
        );

        assert!(out.contains("&lt;p&gt;"), "{out}");
        assert!(out.contains("&quot;c&quot;"), "{out}");
        assert!(out.contains("&apos;d&apos;"), "{out}");
        // The pre-escaped ampersand must be re-escaped, not passed through.
        assert!(out.contains("&amp;amp;"), "{out}");
    }

    #[test]
    fn document_is_well_formed_and_ordered() {
        let out = render("https://youwin.dev", &[row("abc", "<p>hi</p>", "hi")]);

        assert!(out.starts_with(r#"<?xml version="1.0" encoding="utf-8"?>"#));
        assert!(out.trim_end().ends_with("</feed>"));
        assert_eq!(out.matches("<entry>").count(), 1);
        assert!(out.contains("<id>https://youwin.dev/p/abc</id>"), "{out}");
        assert!(out.contains("<updated>2026-08-09T07:06:39Z</updated>"), "{out}");
    }

    #[test]
    fn empty_feed_still_produces_a_valid_document() {
        let out = render("https://youwin.dev", &[]);
        assert!(out.contains("<updated>"), "{out}");
        assert_eq!(out.matches("<entry>").count(), 0);
    }
}
