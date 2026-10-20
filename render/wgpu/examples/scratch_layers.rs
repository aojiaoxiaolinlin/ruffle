//! GPU regression probe: compare eager/deferred layer images and checkout peaks.
//! Run with `RUFFLE_GPU_EAGER_LAYERS=1` for the old scheduler.
//! Arguments: layer-count width height frames output-directory-or-"-" mode.
//! Modes: layers, complex, complex-bitmap, masks, fallback, mixed, dual-pool, bitmap-lifetime.
//! `compare before after frames` checks images.
//! Elapsed time includes startup/warm-up and optional readback; it is not game FPS.

#[cfg(not(target_family = "wasm"))]
fn main() {
    use ruffle_render::backend::RenderBackend;
    use ruffle_render::commands::{CommandHandler, CommandList, RenderBlendMode};
    use ruffle_render::matrix::Matrix;
    use ruffle_render::quality::StageQuality;
    use ruffle_render_wgpu::backend::WgpuRenderBackend;
    use ruffle_render_wgpu::wgpu;
    use std::time::{Duration, Instant};
    use swf::{BlendMode, Color, Twips};

    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "compare") {
        let tolerance = args.get(5).map_or(0, |v| v.parse::<u8>().unwrap());
        let max_outliers = args.get(6).map_or(0, |v| v.parse::<usize>().unwrap());
        for frame in 0..args[4].parse::<usize>().unwrap() {
            let a = image::open(format!("{}/{frame:03}.png", args[2]))
                .unwrap()
                .into_rgba8();
            let b = image::open(format!("{}/{frame:03}.png", args[3]))
                .unwrap()
                .into_rgba8();
            assert_eq!(a.dimensions(), b.dimensions());
            let changed = a.pixels().zip(b.pixels()).filter(|(a, b)| a != b).count();
            let max_difference = a
                .as_raw()
                .iter()
                .zip(b.as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            let outliers = a
                .pixels()
                .zip(b.pixels())
                .filter(|(a, b)| a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > tolerance))
                .count();
            println!(
                "frame={frame} changed_pixels={changed} max_channel_difference={max_difference} outliers={outliers}"
            );
            let image_width = a.width();
            if max_difference > tolerance {
                for (i, (a, b)) in a
                    .pixels()
                    .zip(b.pixels())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(8)
                {
                    println!(
                        "difference x={} y={} before={a:?} after={b:?}",
                        i as u32 % image_width,
                        i as u32 / image_width
                    );
                }
            }
            assert!(outliers <= max_outliers);
        }
        return;
    }
    let count = args.get(1).map_or(16, |v| v.parse::<usize>().unwrap());
    let width = args.get(2).map_or(640, |v| v.parse::<u32>().unwrap());
    let height = args.get(3).map_or(400, |v| v.parse::<u32>().unwrap());
    let frames = args.get(4).map_or(10, |v| v.parse::<usize>().unwrap());
    let output = args.get(5).map(String::as_str).unwrap_or("-");
    let mode = args.get(6).map(String::as_str).unwrap_or("layers");
    let mut renderer = WgpuRenderBackend::for_offscreen(
        (width, height),
        if std::env::var_os("RUFFLE_PROBE_GLES").is_some() {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::VULKAN
        },
        wgpu::PowerPreference::HighPerformance,
    )
    .unwrap();
    renderer.set_quality(if std::env::var_os("RUFFLE_PROBE_MEDIUM").is_some() {
        StageQuality::Medium
    } else {
        StageQuality::High
    });
    if mode == "bitmap-lifetime" {
        use ruffle_render::bitmap::{Bitmap, BitmapFormat};
        let dimension = std::num::NonZeroU32::new(16).unwrap();
        let cache = renderer
            .create_bitmap_cache_texture(dimension, dimension)
            .unwrap();
        let target = renderer.create_empty_texture(dimension, dimension).unwrap();
        let uploaded = renderer
            .register_bitmap(Bitmap::new(
                16,
                16,
                BitmapFormat::Rgba,
                vec![255u8; 16 * 16 * 4],
            ))
            .unwrap();
        let cache_copy = cache.clone();
        let stats = renderer.bitmap_texture_stats();
        assert_eq!(
            (stats.cache_count, stats.target_count, stats.uploaded_count),
            (1, 1, 1)
        );
        assert_eq!(
            (stats.cache_bytes, stats.target_bytes, stats.uploaded_bytes),
            (1024, 1024, 1024)
        );
        let report = renderer.device().generate_allocator_report().unwrap();
        for prefix in ["CacheAsBitmap ", "Bitmap target ", "Uploaded bitmap "] {
            assert!(
                report
                    .allocations
                    .iter()
                    .any(|allocation| allocation.name.starts_with(prefix))
            );
        }
        drop(cache);
        assert_eq!(renderer.bitmap_texture_stats().cache_count, 1);
        drop(cache_copy);
        assert_eq!(renderer.bitmap_texture_stats().cache_count, 0);
        drop((target, uploaded));
        let stats = renderer.bitmap_texture_stats();
        assert_eq!(
            (stats.cache_count, stats.target_count, stats.uploaded_count),
            (0, 0, 0)
        );
        println!("Bitmap lifetime accounting passed; weak diagnostics retain no textures");
        return;
    }
    println!("{}", renderer.debug_info());
    if output != "-" {
        std::fs::create_dir_all(output).unwrap();
    }
    let start = Instant::now();
    let bitmap = (mode == "complex-bitmap").then(|| {
        use ruffle_render::bitmap::{Bitmap, BitmapFormat};
        let pixels: Vec<u8> = (0..32 * 32)
            .flat_map(|i| {
                let alpha = if i % 3 == 0 { 0 } else { 180 };
                [alpha, alpha / 2, alpha / 3, alpha]
            })
            .collect();
        renderer
            .register_bitmap(Bitmap::new(32, 32, BitmapFormat::Rgba, pixels))
            .unwrap()
    });
    let offscreen = (mode == "dual-pool").then(|| {
        renderer
            .create_empty_texture(
                std::num::NonZeroU32::new((width / 2).max(1)).unwrap(),
                std::num::NonZeroU32::new((height / 2).max(1)).unwrap(),
            )
            .unwrap()
    });
    for frame in 0..frames {
        if let Some(handle) = &offscreen {
            let mut draws = CommandList::new();
            draws.draw_rect(Color::WHITE, Matrix::scale(50.0, 50.0));
            drop(renderer.render_offscreen(
                handle.clone(),
                draws,
                StageQuality::High,
                ruffle_render::bitmap::PixelRegion::for_whole_size(
                    (width / 2).max(1),
                    (height / 2).max(1),
                ),
            ));
        }
        let mut commands = CommandList::new();
        commands.draw_rect(
            Color {
                r: 40,
                g: 70,
                b: 95,
                a: 255,
            },
            Matrix::scale(width as f32, height as f32),
        );
        for layer in 0..count {
            let mut child = CommandList::new();
            let matrix = Matrix::create_box_with_rotation(
                70.0 + (layer % 3) as f32 * 7.0,
                65.0,
                (frame as f32 * 0.03) + layer as f32 * 0.02,
                Twips::from_pixels((layer * 31 % width as usize) as f64 - 10.0),
                Twips::from_pixels((layer * 23 % height as usize) as f64 + 5.0),
            );
            child.draw_rect(
                Color {
                    r: (layer * 37 % 255) as u8,
                    g: 150,
                    b: 210,
                    a: 180,
                },
                matrix,
            );
            if let Some(bitmap) = &bitmap {
                use ruffle_render::bitmap::{PixelRegion, PixelSnapping};
                child.render_bitmap(
                    bitmap.clone(),
                    ruffle_render::transform::Transform {
                        matrix: matrix * Matrix::scale(1.0 / 24.0, 1.0 / 20.0),
                        ..Default::default()
                    },
                    true,
                    PixelSnapping::Always,
                    PixelRegion {
                        x_min: 4,
                        y_min: 3,
                        x_max: 28,
                        y_max: 23,
                    },
                );
            }
            if mode == "masks" {
                let mut mask = CommandList::new();
                mask.draw_rect(Color::WHITE, matrix * Matrix::scale(0.8, 0.75));
                commands.render_alpha_mask(child, mask);
            } else {
                if matches!(mode, "fallback" | "dual-pool") || (mode == "mixed" && layer % 3 == 0) {
                    let mut alpha = CommandList::new();
                    alpha.draw_rect(
                        Color {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 100,
                        },
                        matrix,
                    );
                    child.blend(alpha, RenderBlendMode::Builtin(BlendMode::Alpha));
                }
                let modes = [
                    BlendMode::Layer,
                    BlendMode::Add,
                    BlendMode::Screen,
                    BlendMode::Multiply,
                    BlendMode::Overlay,
                    BlendMode::Lighten,
                    BlendMode::Darken,
                    BlendMode::Difference,
                    BlendMode::Invert,
                    BlendMode::HardLight,
                ];
                let complex_modes = [
                    BlendMode::Multiply,
                    BlendMode::Lighten,
                    BlendMode::Darken,
                    BlendMode::Difference,
                    BlendMode::Invert,
                    BlendMode::Overlay,
                    BlendMode::HardLight,
                ];
                let blend = if matches!(mode, "complex" | "complex-bitmap") {
                    complex_modes[layer % complex_modes.len()]
                } else if matches!(mode, "fallback" | "mixed" | "dual-pool") {
                    BlendMode::Layer
                } else {
                    modes[layer % modes.len()]
                };
                commands.blend(child, RenderBlendMode::Builtin(blend));
            }
        }
        renderer.submit_frame(Color::BLACK, commands, vec![]);
        let (main, offscreen) = renderer.scratch_texture_stats();
        for stats in [main.blend_regions, offscreen.blend_regions] {
            assert_eq!(
                stats.prepared,
                stats.cropped + stats.fallback.iter().sum::<u64>()
            );
            assert!(stats.cropped <= stats.candidates);
            assert!(stats.actual_pixels <= stats.original_pixels);
        }
        if matches!(mode, "complex" | "complex-bitmap") {
            assert_eq!(
                main.blend_regions.prepared, count as u64,
                "Deferred layers were double-counted"
            );
            assert_eq!(main.blend_regions.candidates, count as u64);
            if std::env::var_os("RUFFLE_GPU_FULL_BLEND_TARGETS").is_some() {
                assert_eq!(main.blend_regions.cropped, 0);
            } else {
                assert!(main.blend_regions.cropped > 0);
            }
        }
        println!("blend_regions={:?}", main.blend_regions);
        assert!(main.peak_checked_out_bytes >= u64::from(width) * u64::from(height) * 4);
        let budget = renderer.scratch_texture_budget();
        assert!(
            main.owned_bytes.saturating_sub(main.checked_out_bytes)
                + offscreen
                    .owned_bytes
                    .saturating_sub(offscreen.checked_out_bytes)
                <= budget,
            "Idle textures exceeded the shared budget"
        );
        println!(
            "frame={frame} peak_bytes={} owned_bytes={} created={} created_bytes={} deferred={} fallback={}",
            main.peak_checked_out_bytes,
            main.owned_bytes,
            main.created_textures + offscreen.created_textures,
            main.created_bytes + offscreen.created_bytes,
            main.deferred_layers,
            main.fallback_layers
        );
        if mode == "dual-pool" {
            println!(
                "offscreen_peak={} offscreen_owned={} offscreen_created={}",
                offscreen.peak_checked_out_bytes, offscreen.owned_bytes, offscreen.created_textures,
            );
            if frame > 1 && budget / 2 >= offscreen.peak_checked_out_bytes {
                assert_eq!(
                    offscreen.created_textures, 0,
                    "Main pool starved offscreen reuse"
                );
            }
        }
        if matches!(mode, "layers" | "masks")
            && std::env::var_os("RUFFLE_GPU_EAGER_LAYERS").is_none()
        {
            assert!(
                main.peak_checked_out_bytes <= u64::from(width) * u64::from(height) * 4 * 12,
                "Independent sibling textures accumulated"
            );
            assert_eq!(main.deferred_layers, count as u64);
            if frame > 1
                && budget >= u64::from(width) * u64::from(height) * 4 * 12
                && (mode == "masks" || std::env::var_os("RUFFLE_GPU_FULL_BLEND_TARGETS").is_some())
            {
                assert_eq!(
                    main.created_textures, 0,
                    "Warm scratch targets were recreated"
                );
            }
        }
        if output != "-" {
            renderer
                .capture_frame()
                .unwrap()
                .save(format!("{output}/{frame:03}.png"))
                .unwrap();
        }
    }
    renderer
        .device()
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    println!("elapsed_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    if let Some(report) = renderer.device().generate_allocator_report() {
        println!("allocator={report:.8?}");
    }
}

#[cfg(target_family = "wasm")]
fn main() {}
