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
    /// Prefills the header's search box. Set on `/search` so a result page shows
    /// what produced it, and refining a search does not mean retyping it.
    pub search: &'a str,
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
            search: "",
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

                // The .ico carries 16/32/48 and is what a browser guesses at
                // when told nothing; the PNGs are listed so one that would
                // rather have a PNG does not have to unpack an icon container
                // to get it. All three come out of web/scripts/generate-icons.mjs,
                // from the same scene as the app icons.
                link rel="icon" href="/favicon.ico" sizes="any";
                link rel="icon" type="image/png" sizes="32x32" href="/favicon-32x32.png";
                link rel="icon" type="image/png" sizes="16x16" href="/favicon-16x16.png";
                link rel="alternate" type="application/atom+xml" title="youwin.dev" href="/feed.xml";
            }
            body class="min-h-dvh" {
                div class="mx-auto flex min-h-dvh max-w-2xl flex-col px-4" {
                    header class="border-b border-base-300 py-6" {
                        div class="flex items-baseline justify-between gap-4" {
                            a href="/" class="text-lg font-medium no-underline" { "youwin.dev" }
                            // Wraps rather than scrolls: at 320px these five sit
                            // on two lines, which is fine, where a nowrap row
                            // would push `github` off the edge.
                            nav class="flex flex-wrap justify-end gap-x-4 gap-y-1 text-sm text-secondary" {
                                a href="/about" { "about" }
                                a href="/archive" { "archive" }
                                a href="/tags" { "tags" }
                                a href="/feed.xml" { "feed" }
                                a href="https://github.com/you-win" rel="me noopener" { "github" }
                            }
                        }

                        // A GET form, so a search is a URL you can link, bookmark
                        // and go back to — and so the whole site still runs
                        // without a line of JavaScript.
                        form action="/search" method="get" role="search" class="mt-4" {
                            input type="search" name="q" value=(page.search)
                                  placeholder="Search" aria-label="Search posts"
                                  autocomplete="off" spellcheck="false"
                                  // No `text-sm` here: theme.css floors form
                                  // controls at 16px so iOS Safari does not zoom
                                  // on focus, and this input should read at the
                                  // size it will actually render.
                                  class="w-full rounded-field border border-base-300 bg-base-200 \
                                         px-3 py-1.5 placeholder:text-secondary/70";
                        }
                    }

                    main class="flex-1 py-8" { (content) }

                    footer class="flex flex-wrap items-baseline justify-between gap-2 \
                                  border-t border-base-300 py-6 text-sm text-secondary" {
                        span {
                            "Written by "
                            a href="https://github.com/you-win" rel="me noopener" { "you-win" }
                            "."
                        }
                        // In the footer rather than the nav: it is a thing to do
                        // when you have finished reading, not a way to get
                        // somewhere.
                        a href="/random" { "something at random →" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// Every icon the shell links, as a root-absolute path.
    ///
    /// An icon has to be true in three places at once: linked here, present in
    /// the repo's `static/` (which CI copies into the release), and listed in the
    /// Caddyfile's `@root_files`, since Caddy serves those by an explicit set of
    /// paths rather than from a directory. Nothing at runtime connects the three,
    /// and an icon missing from any one of them fails the same silent way — the
    /// tab just shows the browser's default and nobody files a bug for months.
    const LINKED_ICONS: [&str; 3] = ["/favicon.ico", "/favicon-32x32.png", "/favicon-16x16.png"];

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR is crates/server.
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
    }

    fn shell() -> String {
        let assets = Assets {
            css: "/assets/test.css".to_owned(),
        };
        let page = Page::new("t", "d", "https://youwin.dev/".to_owned());
        render(&assets, &page, html! {}).into_string()
    }

    #[test]
    fn the_shell_links_every_icon() {
        let html = shell();
        for icon in LINKED_ICONS {
            assert!(
                html.contains(&format!(r#"href="{icon}""#)),
                "{icon} is not linked in the document head:\n{html}"
            );
        }
    }

    #[test]
    fn every_linked_icon_ships_and_is_served() {
        let root = repo_root();

        let caddyfile = std::fs::read_to_string(root.join("deploy/youwin.dev.caddy"))
            .expect("reading deploy/youwin.dev.caddy");
        let root_files = caddyfile
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("@root_files "))
            .expect("the Caddyfile should still declare a @root_files matcher");

        for icon in LINKED_ICONS {
            let on_disk = root.join("static").join(icon.trim_start_matches('/'));
            assert!(
                on_disk.is_file(),
                "{icon} is linked but {} does not exist. \
                 Run `pnpm --dir web run icons`.",
                on_disk.display()
            );
            assert!(
                root_files.split_whitespace().any(|path| path == icon),
                "{icon} is linked but Caddy will not serve it — add it to \
                 @root_files, which currently reads:\n  {root_files}"
            );
        }
    }
}
