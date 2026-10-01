#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = ".." }
//! egui = "0.33"
//! ```

use lissajous::prelude::*;

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.view(move |ctx| {
        md!(
            ctx,
            "# Grid Layout Demo
Flat 12-column grid with coordination-free layout.
Same spans → same pixel widths, every time, in every card."
        );
    });

    // Demonstrate the flat grid with state access at depth.
    let counter = nb.state("counter", 0u32, move |ctx, count| {
        ctx.grid(|g| {
            // Full-width heading
            g.full(|ctx| {
                ctx.label(egui::RichText::new("FLAT GRID").monospace().strong());
            });
            // Two-thirds + one-third split
            g.two_thirds(|ctx| {
                ctx.label("Main column (two-thirds).");
                ctx.label(format!("Counter: {count}"));
                if ctx.button("INCREMENT").clicked() {
                    *count += 1;
                }
            });
            g.third(|ctx| {
                ctx.label(egui::RichText::new("SIDEBAR").monospace().strong());
                ctx.label("One-third column.");
            });
        });
    });

    // State reads at depth — works inside grid, horizontal, collapsing.
    nb.view(move |ctx| {
        ctx.grid(|g| {
            g.full(|ctx| {
                ctx.label(
                    egui::RichText::new("STATE ACCESS AT DEPTH")
                        .monospace()
                        .strong(),
                );
                ctx.horizontal(|ctx| {
                    let count = *counter.read(ctx);
                    ctx.label(format!("Reading counter from inside horizontal: {count}"));
                });
                ctx.collapsing("And inside collapsing too", |ctx| {
                    let count = *counter.read(ctx);
                    ctx.label(format!("Counter is: {count}"));
                });
            })
        });
    });

    // Grid-aligned typography showcase.
    nb.view(move |ctx| {
        md!(
            ctx,
            "# The Modular Grid

A modular grid divides the page into uniform rectangular modules separated
by consistent gutters. Every element — headings, body text, images, and
widgets — aligns to this invisible scaffolding. The result is a layout that
feels ordered without being rigid.

The concept originates from Swiss typographic design of the 1950s and 60s,
pioneered by Josef Müller-Brockmann and others at the Zurich School of
Design. Their insight was simple: constraint liberates. By committing to a
grid, the designer is freed from ad-hoc decisions about placement and can
focus entirely on content hierarchy.

## Vertical Rhythm

In traditional typesetting, the *baseline grid* ensures that lines of text
across adjacent columns align horizontally. This produces a visual harmony
that readers feel even if they cannot articulate it. When baselines drift,
the page looks subtly wrong — like a picture hung slightly crooked.

For screen typography the principle is the same, but the unit changes.
Instead of leading measured in points, we work with a **vertical module** —
a fixed pixel height to which row tops snap. Our module is 12px, matching
the horizontal gutter, so the grid is truly square.

### Font Size Selection

Not every font size produces a line height that is a clean multiple of 12.
With IosevkaGorbie (ascent 965, descent −215, line gap 70, UPM 1000), the
usable sizes are:

- **29px** → 36px row height (3 modules)
- **20px** → 24px row height (2 modules)
- **15px** → 18px row height (1.5 modules)
- **9.5px** → 12px row height (1 module)

These form a natural hierarchy: display headings, section headings, body
copy, and fine print. Every pair of body lines stacks to exactly three
modules — the same height as one display heading.

### Why 1.5 Modules Works

A half-module body line might seem like a compromise, but it is the secret
to the system's coherence. Two body lines equal three modules. Four body
lines equal six. The least common multiple of 12 and 18 is just 36 — so
every second body line *does* land on a full module boundary. In practice,
paragraphs of even modest length realign automatically.

## Horizontal Structure

The horizontal grid divides the 768px notebook column into 12 columns of
51px each, separated by 12px gutters, with 12px edge padding on both sides.
Any span of *n* columns occupies `51n + 12(n−1)` pixels — a linear formula
that makes layout arithmetic trivial.

Common splits include 8+4 (content and sidebar), 4+4+4 (equal thirds),
and 9+3 (wide content with narrow aside). Because every card uses the same
grid, elements across different cards align vertically without any
coordination between their authors.

#### A Note on Irrational Beauty

The column-to-gutter ratio (51:12 = 4.25) is not a clean integer. This is
not a defect. The human eye prefers ratios that are *close to* but not
exactly simple fractions — the same reason the golden ratio (≈1.618) is
more pleasing than 2:1. Our 4.25 ratio sits between 4:1 and 9:2, creating
subtle visual tension that keeps the layout from feeling sterile."
        );
    });

    // Typst math rendering — direct vector glyphs on the Painter.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        ctx.grid(|g| {
            g.full(|ctx| {
                ctx.label(egui::RichText::new("TYPST MATH").monospace().strong());
                ctx.add_space(4.0);

                ctx.label("Inline math:");
                ctx.horizontal(|ctx| {
                    ctx.label("The Euler identity ");
                    ctx.typst_math_inline("e^(i pi) + 1 = 0");
                    ctx.label(" is beautiful.");
                });
                ctx.add_space(4.0);

                ctx.label("Display math — the Gaussian integral:");
                ctx.typst_math_display("integral_(-infinity)^(infinity) e^(-x^2) dif x = sqrt(pi)");
                ctx.add_space(4.0);

                ctx.label("Quadratic formula:");
                ctx.typst_math_display("x = (-b plus.minus sqrt(b^2 - 4a c)) / (2a)");
                ctx.add_space(4.0);

                ctx.label("Maxwell's equations:");
                ctx.typst_math_display(
                    "nabla dot bold(E) = rho / epsilon_0 \\\n\
                 nabla dot bold(B) = 0 \\\n\
                 nabla times bold(E) = -frac(diff bold(B), diff t) \\\n\
                 nabla times bold(B) = mu_0 bold(J) + mu_0 epsilon_0 frac(diff bold(E), diff t)",
                );
            })
        });
    });

    // Typst rich text — headings, lists, emphasis, tables.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(
            ctx,
            "= Typst Document Rendering\n\
             \n\
             This is a *full Typst document* rendered as vector geometry \
             directly on egui's Painter. No SVG, no raster — just \
             tessellated glyph outlines.\n\
             \n\
             == Features\n\
             \n\
             - *Bold* and _italic_ text\n\
             - Nested lists:\n\
               - Sub-item one\n\
               - Sub-item two\n\
             - Inline math: $E = m c^2$\n\
             - Display math:\n\
             $ sum_(k=0)^n binom(n, k) = 2^n $\n\
             \n\
             == A Small Table\n\
             \n\
             #table(\n\
               columns: 3,\n\
               [*Name*], [*Value*], [*Unit*],\n\
               [$c$], [299 792 458], [m/s],\n\
               [$h$], [$6.626 times 10^(-34)$], [J·s],\n\
               [$k_B$], [$1.381 times 10^(-23)$], [J/K],\n\
             )"
        );
    });

    // Typst drawing — exercises curve geometry rendering.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(ctx,
            "#let node(pos, label) = place(dx: pos.at(0) - 20pt, dy: pos.at(1) - 12pt)[\n\
               #box(width: 40pt, height: 24pt, stroke: 1pt + ral-fg, radius: 4pt, align(center + horizon, label))\n\
             ]\n\
             #let arrow(from, to) = {{\n\
               place(line(start: from, end: to, stroke: 1pt + ral-fg))\n\
             }}\n\
             #box(width: 240pt, height: 100pt)[\n\
               #node((40pt, 20pt), [Source])\n\
               #node((120pt, 20pt), [Parse])\n\
               #node((200pt, 20pt), [Eval])\n\
               #node((120pt, 70pt), [Layout])\n\
               #arrow((60pt, 20pt), (100pt, 20pt))\n\
               #arrow((140pt, 20pt), (180pt, 20pt))\n\
               #arrow((120pt, 32pt), (120pt, 58pt))\n\
             ]\n\
             #v(8pt)\n\
             #circle(radius: 16pt, fill: luma(80%), stroke: 1pt + ral-fg)\n\
             #h(8pt)\n\
             #ellipse(width: 48pt, height: 24pt, fill: luma(90%), stroke: 1pt + ral-fg)\n\
             #h(8pt)\n\
             #polygon(fill: luma(85%), stroke: 1pt + ral-fg,\n\
               (0pt, 20pt), (20pt, 0pt), (40pt, 20pt), (20pt, 40pt))"
        );
    });

    // Typst RAL color palette — theme-aware colors from the preamble.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(
            ctx,
            "= RAL Color Palette\n\
             \n\
             The preamble injects the GORBIE RAL palette as named Typst variables.\n\
             Text adapts to the current theme automatically.\n\
             \n\
             == Categorical Colors\n\
             \n\
             #let swatch(name, color) = box(inset: 4pt)[\n\
               #box(width: 12pt, height: 12pt, fill: color, stroke: 0.5pt + ral-fg)\n\
               #h(4pt)\n\
               #text(font: \"IosevkaGorbie\", size: 9pt)[#name]\n\
             ]\n\
             \n\
             #swatch(\"ral-yellow\", ral-yellow)\n\
             #swatch(\"ral-orange\", ral-orange)\n\
             #swatch(\"ral-pink\", ral-pink)\n\
             #swatch(\"ral-red\", ral-red)\n\
             #swatch(\"ral-violet\", ral-violet)\n\
             #swatch(\"ral-blue\", ral-blue)\n\
             #swatch(\"ral-sky\", ral-sky)\n\
             #swatch(\"ral-water\", ral-water)\n\
             #swatch(\"ral-lime\", ral-lime)\n\
             #swatch(\"ral-mint\", ral-mint)\n\
             #swatch(\"ral-green\", ral-green)\n\
             #swatch(\"ral-teal\", ral-teal)\n\
             \n\
             == Theme Colors\n\
             \n\
             #swatch(\"ral-fg\", ral-fg)\n\
             #swatch(\"ral-bg\", ral-bg)\n\
             #swatch(\"ral-accent\", ral-accent)\n\
             \n\
             == Colored Text\n\
             \n\
             Default text adapts to the theme.\n\
             #text(fill: ral-accent)[Accent highlighted] and\n\
             #text(fill: ral-blue)[signal blue] for emphasis.\n\
             \n\
             == Lookup by Number\n\
             \n\
             Any of the 272 RAL Classic colors by number:\n\
             #swatch(\"ral(1003)\", ral(1003))\n\
             #swatch(\"ral(5015)\", ral(5015))\n\
             #swatch(\"ral(6032)\", ral(6032))\n\
             #swatch(\"ral(9005)\", ral(9005))"
        );
    });

    // Rendering features: dash patterns, text stroke, even-odd fill.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(ctx,
            "= Rendering Features\n\
             \n\
             == Dash Patterns\n\
             \n\
             #line(length: 100%, stroke: (paint: ral-fg, thickness: 1pt, dash: \"dashed\"))\n\
             #v(4pt)\n\
             #line(length: 100%, stroke: (paint: ral-accent, thickness: 2pt, dash: \"dotted\"))\n\
             #v(4pt)\n\
             #line(length: 100%, stroke: (paint: ral-blue, thickness: 1.5pt, dash: (array: (6pt, 3pt, 1pt, 3pt), phase: 0pt)))\n\
             #v(8pt)\n\
             #rect(width: 80pt, height: 40pt, stroke: (paint: ral-fg, thickness: 1pt, dash: \"dashed\"))\n\
             \n\
             == Text Stroke\n\
             \n\
             #text(size: 24pt, stroke: 0.5pt + ral-accent)[Outlined]\n\
             #h(8pt)\n\
             #text(size: 24pt, fill: ral-bg, stroke: 1pt + ral-fg)[Hollow]\n\
             \n\
             == Even-Odd Fill Rule\n\
             \n\
             #let star = {{\n\
               import calc: cos, sin, pi\n\
               let pts = ()\n\
               for i in range(5) {{\n\
                 let a = i * 4 * pi / 5 - pi / 2\n\
                 pts.push((30pt + 28pt * cos(a), 30pt + 28pt * sin(a)))\n\
               }}\n\
               pts\n\
             }}\n\
             #polygon(fill: ral-yellow, fill-rule: \"even-odd\", stroke: 1pt + ral-fg, ..star)\n\
             #h(8pt)\n\
             #polygon(fill: ral-blue, fill-rule: \"non-zero\", stroke: 1pt + ral-fg, ..star)"
        );
    });

    // Visual grid reference — shows the 12-column grid and common span patterns.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(ctx,
            "#let b(span, color, label) = box(width: grid-span(span), height: 24pt, fill: color, radius: 3pt,\n\
               align(center + horizon)[#text(fill: ral-bg, weight: \"bold\", size: 9pt)[#label]]\n\
             )\n\
             \n\
             = The 12-Column Grid\n\
             \n\
             #text(size: 9pt, fill: luma(160))[Each block shows its span.\n\
             Gutters between blocks are `grid-gutter` (12pt).]\n\
             \n\
             #v(4pt)\n\
             \n\
             12 individual columns:\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(1, ral-sky, \"1\"), b(1, ral-sky, \"2\"), b(1, ral-sky, \"3\"),\n\
               b(1, ral-sky, \"4\"), b(1, ral-sky, \"5\"), b(1, ral-sky, \"6\"),\n\
               b(1, ral-sky, \"7\"), b(1, ral-sky, \"8\"), b(1, ral-sky, \"9\"),\n\
               b(1, ral-sky, \"10\"), b(1, ral-sky, \"11\"), b(1, ral-sky, \"12\"),\n\
             )\n\
             \n\
             #v(6pt)\n\
             6 + 6 — equal halves (`half`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(6, ral-blue, \"half()\"), b(6, ral-blue, \"half()\"))\n\
             \n\
             #v(6pt)\n\
             4 + 4 + 4 — equal thirds (`third`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(4, ral-teal, \"third()\"), b(4, ral-teal, \"third()\"), b(4, ral-teal, \"third()\"))\n\
             \n\
             #v(6pt)\n\
             3 + 3 + 3 + 3 — equal quarters (`quarter`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(3, ral-green, \"quarter()\"), b(3, ral-green, \"quarter()\"),\n\
               b(3, ral-green, \"quarter()\"), b(3, ral-green, \"quarter()\"))\n\
             \n\
             #v(6pt)\n\
             8 + 4 — content + sidebar (`two_thirds` + `third`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(8, ral-violet, \"two_thirds()\"), b(4, ral-orange, \"third()\"))\n\
             \n\
             #v(6pt)\n\
             9 + 3 — wide content + narrow aside (`three_quarters` + `quarter`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(9, ral-pink, \"three_quarters()\"), b(3, ral-yellow, \"quarter()\"))\n\
             \n\
             #v(6pt)\n\
             4 + \\_ + 4 — skip middle third (`third` + `skip_third` + `third`):\n\
             #stack(dir: ltr, spacing: grid-gutter,\n\
               b(4, ral-red, \"third()\"), b(4, luma(100), \"\"), b(4, ral-red, \"third()\"))\n\
             \n\
             #v(12pt)\n\
             \n\
             == Grid-Aligned Text Columns\n\
             \n\
             Two equal columns using Typst's `columns` with `grid-gutter`:\n\
             \n\
             #columns(2, gutter: grid-gutter)[\n\
               The left column flows naturally. Because the page width and \
               gutter match the GORBIE grid, these columns align perfectly \
               with a 6+6 split — the same as two `half()` cells in the \
               Rust grid API.\n\
               \n\
               #colbreak()\n\
               \n\
               The right column continues here. Every line sits on the \
               same baseline grid as the rest of the notebook. Headings, \
               body text, math, and columns all share one coordinate system.\n\
             ]\n\
             \n\
             Asymmetric 8+4 split using `grid-span`:\n\
             \n\
             #grid(columns: (grid-span(8), grid-span(4)), column-gutter: grid-gutter)[\n\
               This main column spans 8 of 12 grid columns — the classic \
               content-plus-sidebar ratio. The `grid-span(n)` function \
               computes the exact pixel width including inner gutters.\n\
             ][\n\
               #text(fill: ral-accent)[Sidebar]\n\
               \n\
               Narrow aside spanning 4 columns.\n\
             ]"
        );
    });

    // Widget showcase — all GORBIE widgets on the grid.
    #[derive(Clone)]
    struct WidgetState {
        slider_val: f32,
        number_val: f64,
        toggle_on: bool,
        text: String,
        progress: f32,
    }
    nb.state(
        "widgets",
        WidgetState {
            slider_val: 0.42,
            number_val: 3.14,
            toggle_on: false,
            text: String::from("Hello GORBIE"),
            progress: 0.66,
        },
        move |ctx, s| {
            ctx.grid(|g| {
                g.full(|ctx| {
                    ctx.label(egui::RichText::new("WIDGET SHOWCASE").monospace().strong());
                });

                // Slider + Number field side by side
                g.two_thirds(|ctx| {
                    ctx.label("Slider");
                    ctx.slider(&mut s.slider_val, 0.0..=1.0);
                });
                g.third(|ctx| {
                    ctx.label("Number");
                    ctx.number(&mut s.number_val);
                });

                // Toggle + Text field + Progress bar
                g.third(|ctx| {
                    ctx.label("Toggle");
                    ctx.toggle(&mut s.toggle_on, "Active");
                });
                g.third(|ctx| {
                    ctx.label("Text field");
                    ctx.text_field(&mut s.text);
                });
                g.third(|ctx| {
                    ctx.label("Progress");
                    ctx.progress(s.progress);
                });

                // Buttons row
                g.full(|ctx| {
                    ctx.label("Buttons");
                    ctx.horizontal(|ctx| {
                        ctx.button("Regular");
                        ctx.button("Small");
                        if ctx.button("Animate").clicked() {
                            s.progress = (s.progress + 0.1) % 1.1;
                        }
                    });
                });
            });
        },
    );

    // Different grid layouts, all on the same 12-col grid.
    nb.view(move |ctx| {
        ctx.grid(|g| {
            g.full(|ctx| {
                ctx.label(egui::RichText::new("GRID RATIOS").monospace().strong());
                ctx.add_space(8.0);
            });

            // Equal thirds
            for label in ["LEFT", "CENTER", "RIGHT"] {
                g.third(|ctx| {
                    ctx.label(egui::RichText::new(label).monospace());
                    ctx.add(widgets::ProgressBar::new(0.5).text(label).scale_percent());
                });
            }

            // Quarter + half + quarter — wide center
            g.quarter(|ctx| {
                ctx.label("Narrow (quarter)");
            });
            g.half(|ctx| {
                ctx.label("Wide center (half)");
            });
            g.quarter(|ctx| {
                ctx.label("Narrow (quarter)");
            });

            // Three-quarters + quarter — content + aside
            g.three_quarters(|ctx| {
                ctx.label("Main content area (¾)");
            });
            g.quarter(|ctx| {
                ctx.label("Aside (¼)");
            });

            // Furniture demo: third, skip third, third
            g.third(|ctx| {
                ctx.label("Left (third)");
            });
            g.skip_third();
            g.third(|ctx| {
                ctx.label("Right (third)");
            });
        });
    });

    // Links — clickable URL links rendered by Typst.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(
            ctx,
            "= Links\n\
             \n\
             Typst links are clickable:\n\
             #link(\"https://typst.app\")[Typst] and\n\
             #link(\"https://github.com/erlkonig-ai/lissajous\")[Lissajous on GitHub].\n\
             \n\
             Auto-detected URLs also work: https://docs.rs/lissajous"
        );
    });

    // Error diagnostics — Typst errors render inline with source context.
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        md!(ctx, "# Error Diagnostics\nWhen Typst compilation fails, errors render inline with the\noffending source line and a pointer to the problem.");
    });
    #[cfg(feature = "typst")]
    nb.view(move |ctx| {
        typst!(ctx, "This line renders fine, but #unknown-func() does not.");
    });

    // Collapsible sections: one per card, so each can be detached and moved
    // on its own, with the layout nested inside the section.
    nb.view(move |ctx| {
        ctx.section("Configuration", |ctx| {
            ctx.label("A section is the top of its card.");
            ctx.label("Click the header to collapse.");
        });
    });
    nb.view(move |ctx| {
        ctx.section("Output", |ctx| {
            ctx.grid(|g| {
                g.place(6, |ctx| {
                    ctx.label("Grids go inside sections,");
                });
                g.place(6, |ctx| {
                    ctx.label("never sections inside grid cells.");
                });
            });
        });
    });
    nb.view(move |ctx| {
        ctx.section("Diagnostics", |ctx| {
            ctx.label("Each title gets its own RAL color.");
        });
    });

    nb.settled();
}
