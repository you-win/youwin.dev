//! The document shell: head, meta tags, nav, footer.

use maud::{DOCTYPE, Markup, html};

use crate::public::assets::Assets;

/// Everything that varies between pages, gathered so a caller cannot forget
/// half of it. On a shared-origin SPA this was a sentinel-splicing machine; here
/// the OG tags are just fields.
pub struct Page<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// Absolute URL of this page. Canonical link and `og:url`.
    pub canonical: String,
    /// `article` for permalinks, `website` for everything else.
    pub og_type: &'a str,
    /// RFC 3339, permalinks only.
    pub published: Option<String>,
    /// Set for `unlisted` posts, and for anything paginated past page one:
    /// reachable by link, but not worth an index entry.
    pub noindex: bool,
}

impl<'a> Page<'a> {
    pub fn new(title: &'a str, description: &'a str, canonical: String) -> Self {
        Self {
            title,
            description,
            canonical,
            og_type: "website",
            published: None,
            noindex: false,
        }
    }
}

pub fn render(assets: &Assets, page: &Page<'_>, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";

                title { (page.title) }
                meta name="description" content=(page.description);
                link rel="canonical" href=(page.canonical);

                @if page.noindex {
                    meta name="robots" content="noindex, follow";
                }

                // Approximates base-100. The one theme color written outside
                // theme.css, because <meta> cannot read a custom property.
                meta name="theme-color" content="#09120d";

                meta property="og:site_name" content="youwin.dev";
                meta property="og:title" content=(page.title);
                meta property="og:description" content=(page.description);
                meta property="og:url" content=(page.canonical);
                meta property="og:type" content=(page.og_type);
                @if let Some(published) = &page.published {
                    meta property="article:published_time" content=(published);
                }
                meta name="twitter:card" content="summary";

                link rel="stylesheet" href=(assets.css);
                link rel="icon" href="/favicon.ico";
                link rel="alternate" type="application/atom+xml" title="youwin.dev" href="/feed.xml";
            }
            body class="min-h-dvh" {
                div class="mx-auto flex min-h-dvh max-w-2xl flex-col px-4" {
                    header class="flex items-baseline justify-between border-b border-base-300 py-6" {
                        a href="/" class="text-lg font-medium no-underline" { "youwin.dev" }
                        nav class="flex gap-4 text-sm text-secondary" {
                            a href="/about" { "about" }
                            a href="/feed.xml" { "feed" }
                            a href="https://github.com/you-win" rel="me noopener" { "github" }
                        }
                    }

                    main class="flex-1 py-8" { (content) }

                    footer class="border-t border-base-300 py-6 text-sm text-secondary" {
                        "Written by "
                        a href="https://github.com/you-win" rel="me noopener" { "you-win" }
                        "."
                    }
                }
            }
        }
    }
}
