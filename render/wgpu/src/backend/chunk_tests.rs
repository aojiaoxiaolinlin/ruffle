//! Explicit original mixed rendering fixture. Compare saved raw bytes across independent budget arms.
use super::*;
use crate::utils::remove_srgb;
use ruffle_render::backend::null::NullBitmapSource;
use ruffle_render::bitmap::PixelSnapping;
use ruffle_render::commands::{CommandHandler, RenderBlendMode};
use ruffle_render::matrix::Matrix;
use ruffle_render::shape_utils::{DrawCommand as ShapeCommand, DrawPath, FillRule};
use ruffle_render::transform::Transform;
use std::sync::mpsc;
use std::time::Duration;
use swf::{BlendMode, Twips};

const GPU_TIMEOUT: Duration = Duration::from_secs(30);
const CLEAR: Color = Color {
    r: 7,
    g: 19,
    b: 31,
    a: 97,
};

#[derive(Debug)]
struct RawTarget {
    texture: wgpu::Texture,
    backing_format: wgpu::TextureFormat,
    view_format: wgpu::TextureFormat,
    size: wgpu::Extent3d,
    buffer: wgpu::Buffer,
    dimensions: BufferDimensions,
}

#[derive(Debug)]
struct RawFrame(wgpu::TextureView);

impl RenderTargetFrame for RawFrame {
    fn into_view(self) -> wgpu::TextureView {
        self.0
    }

    fn view(&self) -> &wgpu::TextureView {
        &self.0
    }
}

impl RawTarget {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let view_format = remove_srgb(format);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let dimensions = BufferDimensions::new(width as usize, height as usize, format);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Direct stage raw target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            view_formats: &[view_format],
            // Match the swapchain's lack of texture binding / destination reads.
            // COPY_SRC exists only to compare output bytes after submission.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Direct stage raw readback"),
            size: dimensions.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            backing_format: format,
            view_format,
            size,
            buffer,
            dimensions,
        }
    }

    fn raw_bytes(&self, device: &wgpu::Device) -> Vec<u8> {
        let (sender, receiver) = mpsc::channel();
        let slice = self.buffer.slice(..self.dimensions.size());
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(GPU_TIMEOUT),
            })
            .expect("Direct stage GPU completion must be bounded");
        receiver
            .recv_timeout(GPU_TIMEOUT)
            .expect("Readback callback must arrive")
            .expect("Readback must map successfully");
        let mapping = slice.get_mapped_range().expect("Readback must be mapped");
        let mut raw = Vec::with_capacity(self.size.width as usize * self.size.height as usize * 4);
        for row in mapping
            .chunks_exact(self.dimensions.padded_bytes_per_row as usize)
            .take(self.size.height as usize)
        {
            raw.extend_from_slice(&row[..self.dimensions.unpadded_bytes_per_row]);
        }
        drop(mapping);
        self.buffer.unmap();
        assert_eq!(
            raw.len(),
            self.size.width as usize * self.size.height as usize * 4
        );
        raw
    }
}

impl RenderTarget for RawTarget {
    type Frame = RawFrame;

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        *self = Self::new(device, self.backing_format, width, height);
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.view_format
    }

    fn width(&self) -> u32 {
        self.size.width
    }

    fn height(&self) -> u32 {
        self.size.height
    }

    fn get_next_texture(&mut self) -> Option<Self::Frame> {
        Some(RawFrame(self.texture.create_view(
            &wgpu::TextureViewDescriptor {
                format: Some(self.view_format),
                ..Default::default()
            },
        )))
    }

    fn submit<I: IntoIterator<Item = wgpu::CommandBuffer>>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        command_buffers: I,
        _frame: Self::Frame,
    ) -> wgpu::SubmissionIndex {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Direct stage raw copy encoder"),
        });
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
                    bytes_per_row: Some(self.dimensions.padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            self.size,
        );
        queue.submit(command_buffers.into_iter().chain(Some(encoder.finish())))
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Matrix {
    Matrix::create_box(
        width,
        height,
        Twips::from_pixels(x.into()),
        Twips::from_pixels(y.into()),
    )
}

fn transform(x: f32, y: f32, scale_x: f32, scale_y: f32) -> Transform {
    Transform {
        matrix: rect(x, y, scale_x, scale_y),
        ..Default::default()
    }
}

struct Assets {
    bitmap: BitmapHandle,
    stage3d: BitmapHandle,
    shape: ShapeHandle,
    masked_draws: u32,
}

fn register_assets(renderer: &mut WgpuRenderBackend<RawTarget>) -> Assets {
    // Pixels are already premultiplied, with transparent and semitransparent edges.
    let bitmap = renderer
        .register_bitmap(Bitmap::new(
            3,
            3,
            BitmapFormat::Rgba,
            vec![
                0, 0, 0, 0, 39, 11, 7, 63, 113, 57, 23, 127, 7, 45, 21, 63, 197, 173, 71, 255, 47,
                103, 61, 127, 19, 7, 53, 63, 31, 61, 117, 127, 0, 0, 0, 0,
            ],
        ))
        .unwrap();
    let stage3d = renderer
        .register_bitmap(Bitmap::new(
            2,
            2,
            BitmapFormat::Rgba,
            [64, 32, 16, 128].repeat(4),
        ))
        .unwrap();
    let fill = swf::FillStyle::Color(Color {
        r: 173,
        g: 61,
        b: 211,
        a: 139,
    });
    let bounds = swf::Rectangle {
        x_min: Twips::ZERO,
        y_min: Twips::ZERO,
        x_max: Twips::from_pixels(18.0),
        y_max: Twips::from_pixels(17.0),
    };
    let shape = renderer.register_shape(
        DistilledShape {
            paths: vec![DrawPath::Fill {
                style: &fill,
                commands: vec![
                    ShapeCommand::MoveTo(swf::Point::from_pixels(0.25, 0.5)),
                    ShapeCommand::LineTo(swf::Point::from_pixels(17.75, 4.25)),
                    ShapeCommand::LineTo(swf::Point::from_pixels(5.25, 16.75)),
                    ShapeCommand::LineTo(swf::Point::from_pixels(0.25, 0.5)),
                ],
                winding_rule: FillRule::EvenOdd,
            }],
            shape_bounds: bounds,
            edge_bounds: bounds,
            id: 1,
        },
        &NullBitmapSource,
    );
    let transform_capacity =
        renderer.dynamic_transforms.buffer.size() / size_of::<crate::Transforms>() as u64;
    assert!((1..=200).contains(&transform_capacity));
    // More than twice the actual uniform capacity guarantees multiple chunks
    // while the stencil mask is active, even on smaller uniform limits.
    let masked_draws = (transform_capacity * 2 + 17) as u32;
    Assets {
        bitmap,
        stage3d,
        shape,
        masked_draws,
    }
}

#[test]
#[ignore = "Explicit same-binary separate-process completed-chunk pixel matrix"]
fn completed_chunk_pixel_matrix() {
    let output = std::path::PathBuf::from(std::env::var_os("ACTOR_CHUNK_MATRIX_OUTPUT").unwrap());
    assert!(!output.exists());
    std::fs::create_dir(&output).unwrap();
    let backends = match std::env::var("RUFFLE_CHUNK_MATRIX_BACKEND").as_deref() {
        Ok("dx12") => wgpu::Backends::DX12,
        Ok("vulkan") | Err(_) => wgpu::Backends::VULKAN,
        _ => panic!("Unsupported chunk matrix backend"),
    };
    let instance = create_wgpu_instance(backends, Default::default(), None);
    let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
        backends,
        &instance,
        None,
        Default::default(),
    ))
    .unwrap();
    let descriptors = Arc::new(Descriptors::new(instance, adapter, device, queue));
    let errors = descriptors
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    let mut cases = 0;
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        for quality in [StageQuality::Low, StageQuality::High] {
            let target = RawTarget::new(&descriptors.device, format, 83, 67);
            let mut renderer = WgpuRenderBackend::new(descriptors.clone(), target).unwrap();
            renderer.set_quality(quality);
            renderer.set_render_pass_budget(0);
            if std::env::var_os("RUFFLE_CHUNK_MATRIX_LIVE_PROFILER").is_some() {
                assert!(
                    descriptors
                        .device
                        .features()
                        .contains(wgpu_profiler::GpuProfiler::ALL_WGPU_TIMER_FEATURES)
                );
                renderer
                    .profiler
                    .change_settings(wgpu_profiler::GpuProfilerSettings {
                        enable_timer_queries: true,
                        enable_debug_groups: true,
                        ..Default::default()
                    })
                    .unwrap();
            }
            let assets = register_assets(&mut renderer);
            for case in 0..6 {
                let mut commands = CommandList::new();
                commands.draw_rect(
                    Color {
                        r: 97,
                        g: 149,
                        b: 173,
                        a: 177,
                    },
                    rect(0.0, 0.0, 83.0, 67.0),
                );
                if case == 1 {
                    commands.push_mask();
                    commands.draw_rect(Color::WHITE, rect(6.5, 7.5, 55.0, 43.0));
                    commands.activate_mask();
                }
                let mut layers = CommandList::new();
                for n in 0..160 {
                    let mut tile = CommandList::new();
                    tile.draw_rect(
                        Color {
                            r: (31 + n % 191) as u8,
                            g: (83 + n % 137) as u8,
                            b: (127 + n % 103) as u8,
                            a: 93,
                        },
                        rect(
                            2.25 + (n % 13) as f32 * 4.25,
                            3.5 + (n % 11) as f32 * 3.75,
                            17.5,
                            13.25,
                        ),
                    );
                    if case == 2 {
                        let mut inner = CommandList::new();
                        inner.blend(tile, RenderBlendMode::Builtin(BlendMode::Multiply));
                        layers.blend(inner, RenderBlendMode::Builtin(BlendMode::Screen));
                    } else {
                        layers.blend(
                            tile,
                            RenderBlendMode::Builtin(if n % 2 == 0 {
                                BlendMode::Multiply
                            } else {
                                BlendMode::Overlay
                            }),
                        );
                    }
                }
                if case == 3 {
                    let mut alpha = CommandList::new();
                    alpha.draw_rect(
                        Color {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 117,
                        },
                        rect(11.25, 9.5, 43.5, 37.25),
                    );
                    layers.blend(alpha.clone(), RenderBlendMode::Builtin(BlendMode::Alpha));
                    layers.blend(alpha, RenderBlendMode::Builtin(BlendMode::Erase));
                    commands.blend(layers, RenderBlendMode::Builtin(BlendMode::Layer));
                } else if case == 4 {
                    let mut mask = CommandList::new();
                    mask.draw_rect(
                        Color {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 153,
                        },
                        rect(5.0, 8.0, 51.25, 39.75),
                    );
                    commands.render_alpha_mask(layers, mask);
                } else {
                    commands.commands.extend(layers.commands);
                }
                if case == 1 {
                    commands.deactivate_mask();
                    commands.draw_rect(Color::WHITE, rect(6.5, 7.5, 55.0, 43.0));
                    commands.pop_mask();
                }
                commands.render_shape(assets.shape.clone(), transform(58.5, 43.5, 1.0, 1.0));
                commands.render_bitmap(
                    assets.bitmap.clone(),
                    transform(4.0, 4.0, 3.0, 3.0),
                    true,
                    PixelSnapping::Never,
                    PixelRegion::for_whole_size(3, 3),
                );
                commands.render_stage3d(assets.stage3d.clone(), transform(62.0, 3.0, 4.0, 4.0));
                if case == 1 {
                    for n in 0..assets.masked_draws {
                        commands.draw_rect(
                            Color {
                                r: 31,
                                g: 71,
                                b: 113,
                                a: 43,
                            },
                            rect(5.0 + (n % 13) as f32, 9.0 + (n % 17) as f32, 3.0, 4.0),
                        );
                    }
                }
                let mut caches = Vec::new();
                if case == 5 {
                    let handle = renderer
                        .create_bitmap_cache_texture(83.try_into().unwrap(), 67.try_into().unwrap())
                        .unwrap();
                    caches.push(BitmapCacheEntry {
                        handle: handle.clone(),
                        commands,
                        clear: Color::TRANSPARENT,
                        filters: vec![Filter::BlurFilter(swf::BlurFilter {
                            blur_x: swf::Fixed16::from_f32(3.0),
                            blur_y: swf::Fixed16::from_f32(5.0),
                            flags: swf::BlurFilterFlags::from_passes(3),
                        })],
                    });
                    commands = CommandList::new();
                    commands.render_bitmap(
                        handle,
                        Transform::default(),
                        true,
                        PixelSnapping::Never,
                        PixelRegion::for_whole_size(83, 67),
                    );
                }
                renderer.submit_frame(CLEAR, commands, caches);
                let raw = renderer.target.raw_bytes(renderer.device());
                assert!(raw.windows(4).any(|v| v != &raw[..4]));
                std::fs::write(
                    output.join(format!("{format:?}-{quality:?}-{case}.rgba")),
                    raw,
                )
                .unwrap();
                cases += 1;
            }
            std::fs::write(
                output.join(format!("{format:?}-{quality:?}.stats")),
                format!("{:?}", renderer.pass_budget_stats_for_probe()),
            )
            .unwrap();
        }
    }
    assert_eq!(cases, 48);
    assert!(futures::executor::block_on(errors.pop()).is_none());
    eprintln!("ACTOR_CHUNK_MATRIX cases={cases} validation_errors=0");
}
