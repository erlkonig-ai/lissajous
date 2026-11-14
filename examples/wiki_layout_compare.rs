//! The live wiki graph under today's force law and under candidate laws, side
//! by side, as PNGs, with the numbers that say what the pictures show.
//!
//! No window, no GPU: the layout is `lissajous::graph::ForceLayout` on the CPU,
//! and the picture is drawn by a small rasteriser below into the `png` crate
//! the library already depends on, framed and sized the way the wiki widget
//! frames and sizes it (`GraphViewport::fit` with a 40-point margin, marks of
//! `MARK * clamp(zoom, 0.45, 1.5)` grown logarithmically with degree, 0.5-point
//! edges, a one-point outline), at two pixels per point.
//!
//! ```ignore
//! cargo run --release --example wiki_layout_compare -- <nodes.tsv> <edges.tsv> <out_dir>
//! ```
//!
//! The TSVs come from faculties' `wiki_graph_export` example, which reads the
//! pile through the wiki widget's own `WikiGraph::read`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lissajous::graph::{ForceLayout, LayoutParams, LayoutStats, MARK};
use lissajous::themes::colorhash;

/// Points of the drawing rect, and pixels per point. 738 x 773 points at 2x is
/// the size of the screenshot this is compared against.
const RECT: [f32; 2] = [738.0, 773.0];
const PPP: f32 = 2.0;
/// `LABEL_MARGIN` in the wiki widget.
const MARGIN: f32 = 40.0;
const MIN_ZOOM: f32 = 0.002;
const MAX_ZOOM: f32 = 10.0;
/// Hard cap on solver steps per variant.
const STEP_CAP: u64 = 30_000;
/// Steps run past quiet, to check that quiet was not a frozen half-settle.
const AFTER: u64 = 5_000;

const BACKGROUND: [f32; 3] = [28.0, 28.0, 28.0];
const OUTLINE: [f32; 3] = [148.0, 150.0, 145.0];
const EDGE: [f32; 3] = [148.0, 150.0, 145.0];
const ARCHIVED: [f32; 3] = [102.0, 102.0, 102.0];

struct Node {
    entry_key: Vec<u8>,
    archived: bool,
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn read_graph(nodes: &Path, edges: &Path) -> (Vec<Node>, Vec<(u32, u32)>) {
    let text = std::fs::read_to_string(nodes).unwrap();
    let nodes: Vec<Node> = text
        .lines()
        .skip(1)
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            Node {
                entry_key: hex_bytes(cols[2]),
                archived: cols[3] == "1",
            }
        })
        .collect();
    let text = std::fs::read_to_string(edges).unwrap();
    let edges = text
        .lines()
        .skip(1)
        .map(|line| {
            let mut cols = line.split('\t');
            let a = cols.next().unwrap().parse().unwrap();
            let b = cols.next().unwrap().parse().unwrap();
            (a, b)
        })
        .collect();
    (nodes, edges)
}

fn drawn_scale(degree: f32) -> f32 {
    (1.0 + degree.ln() * 0.4).min(3.0)
}

/// `GraphViewport::fit`: zoom, and the world point at the rect's centre.
fn fit(bounds: [f32; 4]) -> (f32, [f32; 2]) {
    let width = (bounds[2] - bounds[0]).max(1.0);
    let height = (bounds[3] - bounds[1]).max(1.0);
    let usable = [
        (RECT[0] - MARGIN * 2.0).max(1.0),
        (RECT[1] - MARGIN * 2.0).max(1.0),
    ];
    let zoom = (usable[0] / width)
        .min(usable[1] / height)
        .clamp(MIN_ZOOM, MAX_ZOOM);
    let centre = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
    (zoom, centre)
}

/// Count unordered pairs closer than `radius[i] + radius[j]`, bucketed on a grid
/// whose cell is the largest possible contact distance.
fn contacts(points: &[[f32; 2]], radius: &[f32]) -> usize {
    let cell = 2.0 * radius.iter().copied().fold(0.0f32, f32::max);
    if cell <= 0.0 {
        return 0;
    }
    let key = |p: [f32; 2]| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (index, point) in points.iter().enumerate() {
        grid.entry(key(*point)).or_default().push(index);
    }
    let mut count = 0;
    for (index, point) in points.iter().enumerate() {
        let (cx, cy) = key(*point);
        for gx in cx - 1..=cx + 1 {
            for gy in cy - 1..=cy + 1 {
                let Some(bucket) = grid.get(&(gx, gy)) else {
                    continue;
                };
                for &other in bucket {
                    if other <= index {
                        continue;
                    }
                    let q = points[other];
                    let d = (point[0] - q[0]).hypot(point[1] - q[1]);
                    if d < radius[index] + radius[other] {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

struct Canvas {
    width: usize,
    height: usize,
    rgb: Vec<[f32; 3]>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            height,
            rgb: vec![BACKGROUND; width * height],
        }
    }

    fn blend(&mut self, x: i64, y: i64, color: [f32; 3], alpha: f32) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 || alpha <= 0.0 {
            return;
        }
        let alpha = alpha.min(1.0);
        let pixel = &mut self.rgb[y as usize * self.width + x as usize];
        for channel in 0..3 {
            pixel[channel] = pixel[channel] * (1.0 - alpha) + color[channel] * alpha;
        }
    }

    /// An anti-aliased line `width` pixels wide, walked along its major axis
    /// so its cost is its length, not its bounding box.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 3], alpha: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        if length < 1e-3 {
            return;
        }
        let half = width * 0.5;
        let steep = dy.abs() > dx.abs();
        let (a, b) = if steep {
            ([a[1], a[0]], [b[1], b[0]])
        } else {
            (a, b)
        };
        let (a, b) = if a[0] > b[0] { (b, a) } else { (a, b) };
        let slope = (b[1] - a[1]) / (b[0] - a[0]).max(1e-6);
        // Perpendicular distance per unit of minor-axis offset.
        let cos = 1.0 / (1.0 + slope * slope).sqrt();
        let reach = (half + 1.0) / cos;
        let start = a[0].floor() as i64;
        let end = b[0].ceil() as i64;
        for major in start..=end {
            let centre = major as f32 + 0.5;
            let along = (centre - a[0]).clamp(0.0, b[0] - a[0]);
            let minor_line = a[1] + slope * along;
            // Fade the caps over half a pixel.
            let cap = ((centre - a[0] + 0.5).min(b[0] - centre + 0.5)).clamp(0.0, 1.0);
            let low = (minor_line - reach).floor() as i64;
            let high = (minor_line + reach).ceil() as i64;
            for minor in low..=high {
                let distance = ((minor as f32 + 0.5) - minor_line).abs() * cos;
                let coverage = (half + 0.5 - distance).clamp(0.0, 1.0) * cap;
                if steep {
                    self.blend(minor, major, color, coverage * alpha);
                } else {
                    self.blend(major, minor, color, coverage * alpha);
                }
            }
        }
    }

    /// A filled disc with an outline centred on its rim, as egui draws
    /// `draw_node` followed by `circle_stroke`.
    fn disc(&mut self, c: [f32; 2], r: f32, fill: [f32; 3], stroke: f32, outline: [f32; 3]) {
        let reach = r + stroke;
        let (x0, x1) = ((c[0] - reach).floor() as i64, (c[0] + reach).ceil() as i64);
        let (y0, y1) = ((c[1] - reach).floor() as i64, (c[1] + reach).ceil() as i64);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - c[0]).hypot(y as f32 + 0.5 - c[1]);
                let inside = (r + 0.5 - d).clamp(0.0, 1.0);
                self.blend(x, y, fill, inside);
                let ring = (stroke * 0.5 + 0.5 - (d - r).abs()).clamp(0.0, 1.0);
                self.blend(x, y, outline, ring);
            }
        }
    }

    fn write_png(&self, path: &Path) {
        let bytes: Vec<u8> = self
            .rgb
            .iter()
            .flat_map(|p| p.map(|v| v.round().clamp(0.0, 255.0) as u8))
            .collect();
        write_rgb(path, self.width, self.height, &bytes);
    }

    /// Half-size copy, by 2x2 box filter, for the contact sheet.
    fn halved(&self) -> Canvas {
        let (width, height) = (self.width / 2, self.height / 2);
        let mut out = Canvas::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0f32; 3];
                for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = self.rgb[(2 * y + oy) * self.width + 2 * x + ox];
                    for channel in 0..3 {
                        sum[channel] += p[channel] * 0.25;
                    }
                }
                out.rgb[y * width + x] = sum;
            }
        }
        out
    }
}

fn write_rgb(path: &Path, width: usize, height: usize, bytes: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(bytes).unwrap();
}

/// What one picture shows, measured on the positions it was drawn from.
struct Frame {
    radius: f32,
    ring_fraction: f32,
    isolated: usize,
    world_overlaps: usize,
    screen_overlaps: usize,
    zoom: f32,
    image: PathBuf,
}

struct Outcome {
    name: &'static str,
    quiet_at: Option<u64>,
    seconds: f64,
    at_quiet: Frame,
    settled: Frame,
}

fn bounding_radius(points: &[[f32; 2]]) -> f32 {
    let n = points.len().max(1) as f32;
    let cx = points.iter().map(|p| p[0]).sum::<f32>() / n;
    let cy = points.iter().map(|p| p[1]).sum::<f32>() / n;
    points
        .iter()
        .map(|p| (p[0] - cx).hypot(p[1] - cy))
        .fold(0.0f32, f32::max)
}

fn frame(
    layout: &ForceLayout,
    nodes: &[Node],
    edges: &[(u32, u32)],
    image: PathBuf,
) -> (Frame, Canvas) {
    let points: Vec<[f32; 2]> = layout.positions().to_vec();
    let degree: Vec<f32> = (0..nodes.len()).map(|i| layout.degree(i)).collect();

    // The ring: unlinked nodes beyond the 90th-percentile radius of linked
    // ones, both measured from the centroid.
    let n = points.len() as f32;
    let cx = points.iter().map(|p| p[0]).sum::<f32>() / n;
    let cy = points.iter().map(|p| p[1]).sum::<f32>() / n;
    let radius_of = |p: [f32; 2]| (p[0] - cx).hypot(p[1] - cy);
    let mut linked: Vec<f32> = (0..points.len())
        .filter(|i| degree[*i] > 1.0)
        .map(|i| radius_of(points[i]))
        .collect();
    linked.sort_by(f32::total_cmp);
    let p90 = linked[(linked.len() * 9) / 10];
    let isolated: Vec<usize> = (0..points.len()).filter(|i| degree[*i] <= 1.0).collect();
    let beyond = isolated
        .iter()
        .filter(|i| radius_of(points[**i]) > p90)
        .count();
    let ring_fraction = beyond as f32 / isolated.len().max(1) as f32;

    // World overlap as briefed: pairs closer than 2 * MARK world units.
    let world_overlaps = contacts(&points, &vec![MARK; points.len()]);

    // What the reader sees: drawn marks at the fit zoom, in screen points.
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in &points {
        bounds[0] = bounds[0].min(p[0]);
        bounds[1] = bounds[1].min(p[1]);
        bounds[2] = bounds[2].max(p[0]);
        bounds[3] = bounds[3].max(p[1]);
    }
    let (zoom, centre) = fit(bounds);
    let mark = MARK * zoom.clamp(0.45, 1.5);
    let screen: Vec<[f32; 2]> = points
        .iter()
        .map(|p| {
            [
                RECT[0] * 0.5 + (p[0] - centre[0]) * zoom,
                RECT[1] * 0.5 + (p[1] - centre[1]) * zoom,
            ]
        })
        .collect();
    let drawn: Vec<f32> = degree.iter().map(|d| mark * drawn_scale(*d)).collect();
    let screen_overlaps = contacts(&screen, &drawn);

    let mut canvas = Canvas::new((RECT[0] * PPP) as usize, (RECT[1] * PPP) as usize);
    let px = |p: [f32; 2]| [p[0] * PPP, p[1] * PPP];
    for &(a, b) in edges {
        let (a, b) = (a as usize, b as usize);
        if a >= screen.len() || b >= screen.len() || a == b {
            continue;
        }
        canvas.line(px(screen[a]), px(screen[b]), 0.5 * PPP, EDGE, 0.55);
    }
    for (index, node) in nodes.iter().enumerate() {
        let fill = if node.archived {
            ARCHIVED
        } else {
            let c = colorhash::ral_categorical(&node.entry_key);
            [c.r() as f32, c.g() as f32, c.b() as f32]
        };
        canvas.disc(px(screen[index]), drawn[index] * PPP, fill, PPP, OUTLINE);
    }
    canvas.write_png(&image);

    let frame = Frame {
        radius: bounding_radius(&points),
        ring_fraction,
        isolated: isolated.len(),
        world_overlaps,
        screen_overlaps,
        zoom,
        image,
    };
    (frame, canvas)
}

fn run(
    name: &'static str,
    params: LayoutParams,
    nodes: &[Node],
    edges: &[(u32, u32)],
    out: &Path,
) -> (Outcome, Canvas, Canvas) {
    let mut layout = ForceLayout::new(nodes.len(), edges, params);
    let started = Instant::now();
    let mut stats: LayoutStats = layout.stats();
    let mut quiet_at = None;
    while stats.steps < STEP_CAP {
        stats = layout.step();
        if stats.quiet {
            quiet_at = Some(stats.steps);
            break;
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    // What the widget shows: it stops repainting at quiet.
    let (at_quiet, quiet_canvas) = frame(&layout, nodes, edges, out.join(format!("{name}.png")));

    // Keep going past quiet, reheated and with the heat schedule off, so the
    // picture is the law's equilibrium rather than wherever the cooled field
    // stopped.
    layout.set_params(LayoutParams {
        cool: 1.0,
        ..params
    });
    let everyone: Vec<u32> = (0..nodes.len() as u32).collect();
    layout.heat(&everyone, 0);
    for _ in 0..AFTER {
        layout.step();
    }
    let (settled, settled_canvas) = frame(
        &layout,
        nodes,
        edges,
        out.join(format!("{name}-settled.png")),
    );

    let outcome = Outcome {
        name,
        quiet_at,
        seconds,
        at_quiet,
        settled,
    };
    (outcome, quiet_canvas, settled_canvas)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (nodes, edges) = read_graph(Path::new(&args[0]), Path::new(&args[1]));
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).unwrap();
    eprintln!("{} nodes, {} edges", nodes.len(), edges.len());

    let atlas = LayoutParams::atlas();
    let variants: Vec<(&'static str, LayoutParams)> = vec![
        ("0-today", LayoutParams::default()),
        (
            "1-atlas-nocollide",
            LayoutParams {
                collide_radius: 0.0,
                ..atlas
            },
        ),
        ("2-atlas", atlas),
        (
            "3-atlas-wide",
            LayoutParams {
                collide_radius: 20.0,
                ..atlas
            },
        ),
        (
            "4-linlog-wide",
            LayoutParams {
                spring_log: true,
                // ln(1 + 40) / 40 of the linear pull at a 40-unit edge:
                // rescaled so a leaf pair still rests near 40 units.
                attraction: atlas.attraction * 40.0 / 41.0f32.ln(),
                collide_radius: 20.0,
                ..atlas
            },
        ),
        (
            "5-atlas-tight",
            LayoutParams {
                // Springs eight times firmer: a leaf pair rests at 14 units
                // against a 40-unit mean spacing, so linked groups condense
                // and the gaps between them open.
                attraction: atlas.attraction * 8.0,
                ..atlas
            },
        ),
    ];

    let mut outcomes = Vec::new();
    let mut thumbs = Vec::new();
    let mut settled_thumbs = Vec::new();
    for (name, params) in variants {
        let (outcome, canvas, settled) = run(name, params, &nodes, &edges, &out);
        eprintln!(
            "{name}: quiet {:?} in {:.1}s | at quiet R {:.0} ring {:.3} world {} screen {} | settled R {:.0} ring {:.3} world {} screen {}",
            outcome.quiet_at,
            outcome.seconds,
            outcome.at_quiet.radius,
            outcome.at_quiet.ring_fraction,
            outcome.at_quiet.world_overlaps,
            outcome.at_quiet.screen_overlaps,
            outcome.settled.radius,
            outcome.settled.ring_fraction,
            outcome.settled.world_overlaps,
            outcome.settled.screen_overlaps,
        );
        outcomes.push(outcome);
        thumbs.push(canvas.halved());
        settled_thumbs.push(settled.halved());
    }

    // Contact sheets: every variant at half size, in one row.
    for (file, thumbs) in [
        ("sheet.png", &thumbs),
        ("sheet-settled.png", &settled_thumbs),
    ] {
        let (tw, th) = (thumbs[0].width, thumbs[0].height);
        let mut sheet = vec![0u8; tw * thumbs.len() * th * 3];
        for (column, thumb) in thumbs.iter().enumerate() {
            for y in 0..th {
                for x in 0..tw {
                    let p = thumb.rgb[y * tw + x];
                    let at = (y * tw * thumbs.len() + column * tw + x) * 3;
                    for channel in 0..3 {
                        sheet[at + channel] = p[channel].round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        write_rgb(&out.join(file), tw * thumbs.len(), th, &sheet);
    }

    let mut table = std::fs::File::create(out.join("metrics.tsv")).unwrap();
    writeln!(
        table,
        "variant\tframe\tquiet_at\tseconds_to_quiet\tradius\tring_fraction\tisolated\tworld_overlaps_2MARK\tscreen_overlaps_at_fit\tfit_zoom\timage"
    )
    .unwrap();
    for o in &outcomes {
        for (label, f) in [("quiet", &o.at_quiet), ("settled+5000", &o.settled)] {
            writeln!(
                table,
                "{}\t{label}\t{}\t{:.2}\t{:.0}\t{:.3}\t{}\t{}\t{}\t{:.4}\t{}",
                o.name,
                o.quiet_at
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "never".into()),
                o.seconds,
                f.radius,
                f.ring_fraction,
                f.isolated,
                f.world_overlaps,
                f.screen_overlaps,
                f.zoom,
                f.image.display()
            )
            .unwrap();
        }
    }
}
