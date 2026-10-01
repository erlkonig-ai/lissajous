use crate::themes::{industrial_dark, industrial_fonts, industrial_light};
use crate::{
    CaptureOptions, CaptureResult, CapturedPng, HeadlessCaptureConfig, HeadlessTheme, NotebookCore,
    NOTEBOOK_COLUMN_WIDTH, NOTEBOOK_MIN_HEIGHT,
};
use eframe::egui;
use egui_wgpu::wgpu;
use std::time::{Duration, Instant};

type HeadlessResult<T> = CaptureResult<T>;
const HEADLESS_PNG_PPI: f32 = 254.0;

/// Shift every clip rect and mesh vertex in `primitives` vertically by
/// `delta` (in logical points). Used during tiled rendering to pull
/// content from a later band of the card into the next tile's
/// texture, so the renderer can keep using a small `screen_descriptor`.
/// Paint callbacks are left untouched — they aren't generally used by
/// the GORBIE widget set, and translating them would require deeper
/// integration with `egui_wgpu`.
fn shift_primitives_y(primitives: &mut [egui::ClippedPrimitive], delta: f32) {
    let shift = egui::vec2(0.0, delta);
    for prim in primitives {
        prim.clip_rect = prim.clip_rect.translate(shift);
        if let egui::epaint::Primitive::Mesh(mesh) = &mut prim.primitive {
            for v in &mut mesh.vertices {
                v.pos.y += delta;
            }
        }
    }
}

pub(super) fn run_headless(
    core: NotebookCore,
    config: HeadlessCaptureConfig,
) -> HeadlessResult<()> {
    let options = CaptureOptions {
        pixels_per_point: config.pixels_per_point,
        settle_timeout: config.settle_timeout,
    };
    options.validate()?;
    std::fs::create_dir_all(&config.output_dir)?;
    capture(core, options, config.theme, &mut |image| {
        std::fs::write(config.output_dir.join(image.filename()), image.bytes)?;
        Ok(())
    })
}

pub(super) fn capture(
    mut core: NotebookCore,
    options: CaptureOptions,
    theme: HeadlessTheme,
    emit: &mut dyn FnMut(CapturedPng) -> CaptureResult<()>,
) -> CaptureResult<()> {
    options.validate()?;
    let mut runner = HeadlessWgpuRunner::new(options, theme)?;
    runner.capture_cards(&mut core, emit)
}

/// A capture that states its own scope. Silence from a headless run used to mean
/// either "every card rendered" or "every card hit the deadline and I emitted
/// whatever I had"; those need to be distinguishable from outside, because the
/// second one writes plausible-looking stubs. Named remedies rather than a bare
/// count, because the count alone sends people to raise the timeout when the
/// cheaper fix is usually the readiness call.
/// How a card's frame loop ended. `Complete` means the notebook said so;
/// `QuietAtDeadline` means it had stopped moving and the clock collected it;
/// `MovingAtDeadline` means it was still changing and the picture may be
/// half-built. Only the last one is worth telling anyone about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Settle {
    Complete,
    QuietAtDeadline,
    MovingAtDeadline,
}

fn unsettled_report(timed_out: &[usize], total: usize, settle_timeout: Duration) -> Option<String> {
    if timed_out.is_empty() {
        return None;
    }
    let shown: Vec<String> = timed_out.iter().take(8).map(|i| i.to_string()).collect();
    let ellipsis = if timed_out.len() > shown.len() {
        format!(", and {} more", timed_out.len() - shown.len())
    } else {
        String::new()
    };
    Some(format!(
        "GORBIE headless: {} of {} card(s) were STILL CHANGING when the {} ms settle \
         timeout expired, so their images may be half-built. Cards: {}{}. Either call \
         `nb.settled()` when the notebook's content is ready, or raise the settle \
         timeout. Cards that had simply stopped moving are not counted here.",
        timed_out.len(),
        total,
        settle_timeout.as_millis(),
        shown.join(", "),
        ellipsis,
    ))
}

fn report_unsettled(timed_out: &[usize], total: usize, settle_timeout: Duration) {
    if let Some(report) = unsettled_report(timed_out, total, settle_timeout) {
        eprintln!("{report}");
    }
}

struct HeadlessWgpuRunner {
    card_width: f32,
    ctx: egui::Context,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    target_format: wgpu::TextureFormat,
    pixels_per_point: f32,
    target: Option<TargetBuffers>,
    time_seconds: f64,
    settle_timeout: Duration,
}

impl HeadlessWgpuRunner {
    fn new(options: CaptureOptions, theme: HeadlessTheme) -> HeadlessResult<Self> {
        let ctx = egui::Context::default();
        // Make headless mode readable by widgets (lissajous::is_headless)
        // — e.g. sections force-open during capture so screenshots
        // always show their contents.
        crate::mark_headless(&ctx);
        ctx.set_fonts(industrial_fonts());
        ctx.set_style_of(egui::Theme::Light, industrial_light());
        ctx.set_style_of(egui::Theme::Dark, industrial_dark());
        // A headless capture does not ask the operating system anything unless
        // explicitly told to. This used to call `dark_light::detect()`
        // unconditionally, which was wrong twice over:
        //
        // 1. On macOS `dark-light` goes through System Events, so a capture run
        //    triggers an automation-permission prompt — and once granted, it
        //    changed the machine's actual appearance (observed live, mid-session,
        //    on the author's desktop, 2026-07-25).
        // 2. It made captures NON-REPRODUCIBLE. The same command produced a
        //    light or a dark PNG depending on what the operator's desktop
        //    happened to be at that moment, so two runs could not be diffed and
        //    the difference attributed to the code.
        //
        // `--theme auto` still offers the old behaviour for anyone who wants a
        // capture that matches their desktop; it is just no longer the silent
        // default.
        let theme = match theme {
            crate::HeadlessTheme::Light => egui::ThemePreference::Light,
            crate::HeadlessTheme::Dark => egui::ThemePreference::Dark,
            crate::HeadlessTheme::Auto => match dark_light::detect() {
                Ok(dark_light::Mode::Light) => egui::ThemePreference::Light,
                Ok(dark_light::Mode::Dark) => egui::ThemePreference::Dark,
                Ok(dark_light::Mode::Unspecified) | Err(_) => egui::ThemePreference::Dark,
            },
        };
        ctx.set_theme(theme);

        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        // Default `Limits::default()` caps `max_texture_dimension_2d`
        // at 8192, which clips tall notebook cards (e.g. compass with
        // many goals, messages with long bodies). Request the
        // adapter's full hardware ceiling — usually 16384 — so the
        // render succeeds for cards that just need a slightly larger
        // single-tile budget. Cards above the hardware limit still
        // need a tile-and-stitch backend.
        let device_desc = wgpu::DeviceDescriptor {
            label: Some("gorbie_headless_device"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&device_desc))?;
        if (NOTEBOOK_COLUMN_WIDTH * options.pixels_per_point).round()
            > device.limits().max_texture_dimension_2d as f32
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "capture width exceeds this GPU's texture limit; lower the scale",
            )
            .into());
        }

        let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let renderer = egui_wgpu::Renderer::new(
            &device,
            target_format,
            egui_wgpu::RendererOptions::default(),
        );

        Ok(Self {
            card_width: NOTEBOOK_COLUMN_WIDTH,
            ctx,
            device,
            queue,
            renderer,
            target_format,
            pixels_per_point: options.pixels_per_point,
            target: None,
            time_seconds: 0.0,
            settle_timeout: options.settle_timeout,
        })
    }

    fn capture_cards(
        &mut self,
        core: &mut NotebookCore,
        emit: &mut dyn FnMut(CapturedPng) -> CaptureResult<()>,
    ) -> HeadlessResult<()> {
        let mut index = 0;
        let mut timed_out: Vec<usize> = Vec::new();
        loop {
            let (mut output, measured_height, outcome) =
                self.run_frame_until_settled(core, index, NOTEBOOK_MIN_HEIGHT)?;
            let mut moving = outcome == Settle::MovingAtDeadline;
            let Some(measured_height) = measured_height else {
                break;
            };

            let mut textures_delta = egui::TexturesDelta::default();
            textures_delta.append(std::mem::take(&mut output.textures_delta));
            let desired_height = measured_height.max(1.0);
            let (mut output, final_height) = if height_close(NOTEBOOK_MIN_HEIGHT, desired_height) {
                (output, desired_height)
            } else {
                let (mut output, measured_height, outcome) =
                    self.run_frame_until_settled(core, index, desired_height)?;
                moving = moving || outcome == Settle::MovingAtDeadline;
                let Some(measured_height) = measured_height else {
                    break;
                };
                textures_delta.append(std::mem::take(&mut output.textures_delta));
                (output, measured_height.max(1.0))
            };

            output.textures_delta = textures_delta;
            if moving {
                timed_out.push(index);
            }
            let tiles = self.render_output(output, egui::vec2(self.card_width, final_height))?;
            emit_images(index, tiles, emit)?;
            index += 1;
        }
        report_unsettled(&timed_out, index, self.settle_timeout);
        Ok(())
    }

    /// Drive frames until the notebook says it is ready, or the settle timeout
    /// expires, and report WHICH happened.
    ///
    /// A card that reaches the deadline is rendered from whatever frame existed
    /// at that instant. Before this outcome the two exits were indistinguishable,
    /// so a 2 s default silently emitted half-built cards and a 45 s one silently
    /// did not, with nothing to tell them apart.
    ///
    /// The deadline is split in two, because most notebooks never call
    /// [`NotebookCtx::settled`] at all and a warning that fires on every capture
    /// is noise rather than signal. If egui requested no repaint and the measured
    /// height held steady, the frame had stopped moving and the deadline merely
    /// collected it; that is `QuietAtDeadline` and says nothing. Only a frame
    /// still in motion when the clock ran out is worth a word.
    ///
    /// Quiescence deliberately does NOT end the loop. A card waiting on an async
    /// read looks quiescent in the gap before its data lands and requests a
    /// repaint, so treating quiet as done would manufacture exactly the stub this
    /// is here to catch. Timing is unchanged; only the report is sharper.
    fn run_frame_until_settled(
        &mut self,
        core: &mut NotebookCore,
        index: usize,
        height: f32,
    ) -> HeadlessResult<(egui::FullOutput, Option<f32>, Settle)> {
        let start = Instant::now();
        let mut textures_delta = egui::TexturesDelta::default();
        let (mut output, mut measured_height) = self.run_frame(core, index, height)?;
        let mut previous_height = measured_height;
        loop {
            textures_delta.append(std::mem::take(&mut output.textures_delta));

            let settled = core.has_settled();
            let quiet = min_repaint_delay(&output) == Duration::MAX
                && match (measured_height, previous_height) {
                    (Some(now), Some(before)) => height_close(now, before),
                    (None, None) => true,
                    _ => false,
                };
            if settled || start.elapsed() >= self.settle_timeout {
                output.textures_delta = textures_delta;
                let outcome = if settled {
                    Settle::Complete
                } else if quiet {
                    Settle::QuietAtDeadline
                } else {
                    Settle::MovingAtDeadline
                };
                return Ok((output, measured_height, outcome));
            }

            let repaint_delay = min_repaint_delay(&output);
            if repaint_delay > Duration::ZERO {
                std::thread::sleep(repaint_delay.min(Duration::from_millis(16)));
            }
            let (next_output, next_height) = self.run_frame(core, index, height)?;
            output = next_output;
            previous_height = measured_height;
            measured_height = next_height;
        }
    }

    fn run_frame(
        &mut self,
        core: &mut NotebookCore,
        index: usize,
        height: f32,
    ) -> HeadlessResult<(egui::FullOutput, Option<f32>)> {
        let screen_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(self.card_width, height));
        let mut raw_input = egui::RawInput {
            screen_rect: Some(screen_rect),
            max_texture_side: Some(self.device.limits().max_texture_dimension_2d as usize),
            time: Some(self.time_seconds),
            ..Default::default()
        };
        {
            let viewport = raw_input
                .viewports
                .entry(raw_input.viewport_id)
                .or_default();
            viewport.native_pixels_per_point = Some(self.pixels_per_point);
            viewport.inner_rect = Some(screen_rect);
            viewport.outer_rect = Some(screen_rect);
        }
        self.time_seconds += f64::from(raw_input.predicted_dt);

        let mut notebook = core.build_notebook();
        let mut measured_height: Option<f32> = None;
        let output = self.ctx.run_ui(raw_input, |ctx| {
            measured_height = core.draw_card(ctx, &mut notebook, index, self.card_width);
        });
        Ok((output, measured_height))
    }

    fn render_output(
        &mut self,
        output: egui::FullOutput,
        size_points: egui::Vec2,
    ) -> HeadlessResult<Vec<RenderedImage>> {
        let egui::FullOutput {
            textures_delta,
            shapes,
            pixels_per_point,
            ..
        } = output;
        let width = (size_points.x * pixels_per_point).round().max(1.0) as u32;
        let full_height = (size_points.y * pixels_per_point).round().max(1.0) as u32;

        for (id, delta) in &textures_delta.set {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        // Cards taller than `tile_max_height` are split into vertical
        // tiles. The shapes are tessellated once; between tiles, every
        // clip rect and mesh vertex is shifted upward by the previous
        // tile's height so the renderer sees them landing inside a
        // tile-sized texture with its own `screen_descriptor`. This
        // produces multiple PNGs per card — also handy for human
        // readers (and LLM ingesters) who'd rather not load a
        // 200-megapixel single image.
        //
        // The cap is well below the GPU's `max_texture_dimension_2d`
        // (typically 16384) so each tile stays browseable on its own.
        const TARGET_TILE_HEIGHT: u32 = 4000;
        let gpu_max = self.device.limits().max_texture_dimension_2d;
        let tile_max_height = TARGET_TILE_HEIGHT.min(gpu_max).min(full_height).max(1);
        let mut clipped_primitives = self.ctx.tessellate(shapes, pixels_per_point);

        let mut tiles = Vec::new();
        let mut y_cursor: u32 = 0;
        let mut tile_offset_points: f32 = 0.0;
        while y_cursor < full_height {
            let this_tile_height = (full_height - y_cursor).min(tile_max_height);

            // Shift primitives up by the previous tile's height (not on
            // the first iteration). `tile_offset_points` accumulates
            // across tiles so the second tile sees vertices already at
            // (orig - tile_max_height_points), the third at (orig - 2 *
            // tile_max_height_points), etc.
            if y_cursor > 0 {
                let shift = -(tile_max_height as f32) / pixels_per_point;
                shift_primitives_y(&mut clipped_primitives, shift);
                tile_offset_points += -shift;
            }
            let _ = tile_offset_points; // currently informational only

            self.ensure_target(width, this_tile_height)?;
            let screen_descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, this_tile_height],
                pixels_per_point,
            };

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("gorbie_headless_encoder"),
                });
            let mut callbacks = self.renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &clipped_primitives,
                &screen_descriptor,
            );

            let clear = color32_to_wgpu(self.ctx.global_style().visuals.window_fill);
            let target = self.target.as_ref().expect("target ensured");
            {
                let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gorbie_headless_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    multiview_mask: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let mut render_pass = render_pass.forget_lifetime();
                self.renderer
                    .render(&mut render_pass, &clipped_primitives, &screen_descriptor);
            }
            target.copy_to_buffer(&mut encoder);

            callbacks.push(encoder.finish());
            self.queue.submit(callbacks);
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

            let pixels = target.readback(&self.device)?;
            tiles.push(RenderedImage {
                width,
                height: this_tile_height,
                pixels,
            });

            y_cursor += this_tile_height;
        }

        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
        Ok(tiles)
    }

    fn ensure_target(&mut self, width: u32, height: u32) -> HeadlessResult<()> {
        let needs_resize = self.target.as_ref().map_or(true, |target| {
            target.dims.width != width || target.dims.height != height
        });
        if needs_resize {
            self.target = Some(TargetBuffers::new(
                &self.device,
                width,
                height,
                self.target_format,
            ));
        }
        Ok(())
    }
}

struct TargetBuffers {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    dims: BufferDimensions,
}

impl TargetBuffers {
    fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let dims = BufferDimensions::new(width, height);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gorbie_headless_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[format],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gorbie_headless_readback"),
            size: dims.buffer_size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            buffer,
            dims,
        }
    }

    fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        let size = wgpu::Extent3d {
            width: self.dims.width,
            height: self.dims.height,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.dims.padded_bytes_per_row),
                    rows_per_image: Some(self.dims.height),
                },
            },
            size,
        );
    }

    fn readback(&self, device: &wgpu::Device) -> HeadlessResult<Vec<u8>> {
        let buffer_slice = self.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        receiver
            .recv()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "map channel closed"))??;

        let data = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity(
            self.dims.unpadded_bytes_per_row as usize * self.dims.height as usize,
        );
        // Strip row padding required by wgpu's copy alignment.
        for chunk in data.chunks(self.dims.padded_bytes_per_row as usize) {
            pixels.extend_from_slice(&chunk[..self.dims.unpadded_bytes_per_row as usize]);
        }
        drop(data);
        self.buffer.unmap();
        Ok(pixels)
    }
}

#[derive(Clone, Copy)]
struct BufferDimensions {
    width: u32,
    height: u32,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
}

impl BufferDimensions {
    fn new(width: u32, height: u32) -> Self {
        let bytes_per_pixel = 4;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row =
            ((unpadded_bytes_per_row + alignment - 1) / alignment) * alignment;
        Self {
            width,
            height,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
        }
    }

    fn buffer_size(&self) -> u64 {
        self.padded_bytes_per_row as u64 * self.height as u64
    }
}

struct RenderedImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn emit_images(
    card_index: usize,
    images: Vec<RenderedImage>,
    emit: &mut dyn FnMut(CapturedPng) -> CaptureResult<()>,
) -> CaptureResult<()> {
    let tile_count = images.len();
    for (tile_index, image) in images.into_iter().enumerate() {
        let mut bytes = Vec::new();
        write_png_rgba(&mut bytes, image.width, image.height, &image.pixels)?;
        emit(CapturedPng {
            card_index,
            tile_index,
            tile_count,
            width: image.width,
            height: image.height,
            bytes,
        })?;
    }
    Ok(())
}

fn write_png_rgba(
    destination: impl std::io::Write,
    width: u32,
    height: u32,
    data: &[u8],
) -> std::io::Result<()> {
    let mut encoder = png::Encoder::new(destination, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let pixels_per_meter = (HEADLESS_PNG_PPI / 0.0254).round().max(1.0) as u32;
    encoder.set_pixel_dims(Some(png::PixelDimensions {
        xppu: pixels_per_meter,
        yppu: pixels_per_meter,
        unit: png::Unit::Meter,
    }));
    let mut writer = encoder
        .write_header()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    writer
        .write_image_data(data)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
}

fn color32_to_wgpu(color: egui::Color32) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color.r()) / 255.0,
        g: f64::from(color.g()) / 255.0,
        b: f64::from(color.b()) / 255.0,
        a: f64::from(color.a()) / 255.0,
    }
}

fn height_close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.5
}

fn min_repaint_delay(output: &egui::FullOutput) -> Duration {
    output
        .viewport_output
        .values()
        .map(|viewport| viewport.repaint_delay)
        .min()
        .unwrap_or(Duration::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: [u8; 4]) -> RenderedImage {
        RenderedImage {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        }
    }

    #[test]
    fn resident_png_preserves_pixels_dimensions_density_and_tile_order() {
        let pixels = [[10, 20, 30, 255], [40, 50, 60, 0]];
        let mut captures = Vec::new();
        emit_images(2, pixels.into_iter().map(pixel).collect(), &mut |capture| {
            captures.push(capture);
            Ok(())
        })
        .unwrap();
        assert_eq!(captures.len(), 2);
        for (tile_index, (capture, expected)) in captures.iter().zip(pixels).enumerate() {
            assert_eq!(capture.card_index, 2);
            assert_eq!(capture.tile_index, tile_index);
            assert_eq!(capture.tile_count, 2);
            assert_eq!((capture.width, capture.height), (1, 1));
            let mut reader = png::Decoder::new(std::io::Cursor::new(&capture.bytes))
                .read_info()
                .unwrap();
            let dimensions = reader.info().pixel_dims.unwrap();
            assert_eq!(dimensions.xppu, 10_000);
            assert_eq!(dimensions.yppu, 10_000);
            assert_eq!(dimensions.unit, png::Unit::Meter);
            let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut buffer).unwrap();
            assert_eq!((info.width, info.height), (1, 1));
            assert_eq!(info.color_type, png::ColorType::Rgba);
            assert_eq!(buffer[..info.buffer_size()], expected);

            // The file consumer gets exactly the existing encoder's bytes.
            let mut direct = Vec::new();
            write_png_rgba(&mut direct, 1, 1, &expected).unwrap();
            assert_eq!(capture.bytes, direct);
        }
    }

    /// Silence has to mean "every card rendered", never "every card hit the
    /// deadline and I wrote whatever I had". Before the settled flag those were
    /// the same observable, which is how a 2 s default shipped 2 px stubs that
    /// looked like pictures.
    #[test]
    fn a_clean_capture_says_nothing_and_a_timed_out_one_names_its_cards() {
        assert!(unsettled_report(&[], 3375, Duration::from_millis(2000)).is_none());

        let report = unsettled_report(&[4, 9], 3375, Duration::from_millis(2000))
            .expect("a timed-out capture must report itself");
        assert!(report.contains("2 of 3375"), "{report}");
        assert!(report.contains("STILL CHANGING"), "{report}");
        assert!(report.contains("2000 ms"), "{report}");
        assert!(report.contains("4, 9"), "{report}");
        // Both remedies, because the count alone sends people to the timeout
        // when the readiness call is usually the cheaper fix.
        assert!(report.contains("nb.settled()"), "{report}");
        assert!(report.contains("raise"), "{report}");
    }

    /// A run where everything timed out must stay one line, and must still say
    /// how many it is hiding -- an instrument that truncates silently is the
    /// thing this whole change exists to stop.
    #[test]
    fn a_long_timeout_list_truncates_but_states_how_many_it_dropped() {
        let all: Vec<usize> = (0..3375).collect();
        let report = unsettled_report(&all, 3375, Duration::from_millis(45_000)).unwrap();
        assert!(report.contains("3375 of 3375"), "{report}");
        assert!(report.contains("45000 ms"), "{report}");
        assert!(report.contains("0, 1, 2, 3, 4, 5, 6, 7"), "{report}");
        assert!(report.contains("and 3367 more"), "{report}");
        assert_eq!(report.lines().count(), 1, "{report}");
    }

    #[test]
    fn first_consumer_error_stops_without_retry_or_encoding_later_tiles() {
        let mut calls = 0;
        let error = emit_images(
            0,
            vec![
                pixel([0, 0, 0, 255]),
                RenderedImage {
                    width: 1,
                    height: 1,
                    pixels: Vec::new(), // Would fail PNG encoding if reached.
                },
            ],
            &mut |_| {
                calls += 1;
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "consumer closed").into())
            },
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.to_string().contains("consumer closed"));
    }

    #[test]
    fn encoding_error_is_not_delivered_as_an_image() {
        let mut calls = 0;
        let result = emit_images(
            0,
            vec![RenderedImage {
                width: 1,
                height: 1,
                pixels: Vec::new(),
            }],
            &mut |_| {
                calls += 1;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, 0);
    }
}
