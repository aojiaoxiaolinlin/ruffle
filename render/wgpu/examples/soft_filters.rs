//! Native GPU probe for glow/shadow blur resolution and exact BitmapData filters.
//! Arguments: percent (100/75/50), output PNG. Uses Vulkan by default; set
//! RUFFLE_PROBE_GLES to exercise GLES when a compatible desktop adapter exists.
#[cfg(not(target_family = "wasm"))]
fn main() {
    use ruffle_render::backend::{BitmapCacheEntry, RenderBackend};
    use ruffle_render::bitmap::{Bitmap, BitmapFormat, PixelRegion, PixelSnapping};
    use ruffle_render::commands::{CommandHandler, CommandList};
    use ruffle_render::filters::Filter;
    use ruffle_render::matrix::Matrix;
    use ruffle_render::transform::Transform;
    use ruffle_render_wgpu::{backend::WgpuRenderBackend, wgpu};
    use std::num::NonZeroU32;
    use swf::{Color, Fixed8, Fixed16, GlowFilter, GlowFilterFlags, Twips};

    let args: Vec<_> = std::env::args().collect();
    let percent = args.get(1).map_or(50, |s| s.parse::<u32>().unwrap());
    let backends = if std::env::var_os("RUFFLE_PROBE_GLES").is_some() {
        wgpu::Backends::GL
    } else {
        wgpu::Backends::VULKAN
    };
    let mut renderer = WgpuRenderBackend::for_offscreen(
        (1024, 512),
        backends,
        wgpu::PowerPreference::HighPerformance,
    )
    .unwrap();
    assert!(renderer.set_soft_filter_scale(percent));
    assert!(!renderer.set_soft_filter_scale(0));
    let glow = Filter::GlowFilter(GlowFilter {
        color: Color {
            r: 255,
            g: 100,
            b: 20,
            a: 255,
        },
        blur_x: Fixed16::from_f32(24.0),
        blur_y: Fixed16::from_f32(24.0),
        strength: Fixed8::from_f32(1.0),
        flags: GlowFilterFlags::COMPOSITE_SOURCE | GlowFilterFlags::from_passes(2),
    });
    let shadow = Filter::DropShadowFilter(swf::DropShadowFilter {
        color: Color {
            r: 30,
            g: 150,
            b: 255,
            a: 255,
        },
        blur_x: Fixed16::from_f32(24.0),
        blur_y: Fixed16::from_f32(24.0),
        angle: Fixed16::from_f32(0.6),
        distance: Fixed16::from_f32(16.0),
        strength: Fixed8::from_f32(1.0),
        flags: swf::DropShadowFilterFlags::COMPOSITE_SOURCE
            | swf::DropShadowFilterFlags::from_passes(2),
    });
    let handle = renderer
        .create_bitmap_cache_texture(
            NonZeroU32::new(1024).unwrap(),
            NonZeroU32::new(512).unwrap(),
        )
        .unwrap();
    let mut source = CommandList::new();
    source.draw_rect(
        Color::WHITE,
        Matrix::create_box(
            128.0,
            128.0,
            Twips::from_pixels(448.0),
            Twips::from_pixels(192.0),
        ),
    );
    // A narrow opaque line must remain sharp even with a reduced blur branch.
    source.draw_rect(
        Color::WHITE,
        Matrix::create_box(
            1.0,
            100.0,
            Twips::from_pixels(360.0),
            Twips::from_pixels(200.0),
        ),
    );
    let mut screen = CommandList::new();
    screen.render_bitmap(
        handle.clone(),
        Transform::default(),
        false,
        PixelSnapping::Never,
        PixelRegion::for_whole_size(1024, 512),
    );
    renderer.submit_frame(
        Color::BLACK,
        screen,
        vec![BitmapCacheEntry {
            handle,
            commands: source,
            clear: Color::TRANSPARENT,
            filters: vec![glow.clone(), shadow],
        }],
    );
    let (_, stats) = renderer.scratch_texture_stats();
    assert_eq!(stats.soft_blur.requests, 2);
    assert_eq!(stats.soft_blur.reduced, if percent < 100 { 2 } else { 0 });
    println!("soft_blur={:?}", stats.soft_blur);
    let image = renderer.capture_frame().unwrap();
    assert_eq!(image.get_pixel(360, 240).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(500, 240).0, [255, 255, 255, 255]);
    if let Some(path) = args.get(2) {
        image.save(path).unwrap();
    }
    println!(
        "soft_percent={percent} offscreen_peak_bytes={} offscreen_created_bytes={}",
        stats.peak_checked_out_bytes, stats.created_bytes
    );

    // Script-visible BitmapData.applyFilter must be pixel-identical regardless
    // of the display filter setting, including a source subrectangle + offset.
    let mut pixels = vec![0; 256 * 256 * 4];
    for y in 64..192 {
        for x in 64..192 {
            pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].fill(255);
        }
    }
    let input = renderer
        .register_bitmap(Bitmap::new(256, 256, BitmapFormat::Rgba, pixels))
        .unwrap();
    let mut outputs = Vec::new();
    for scale in [100, 50] {
        renderer.set_soft_filter_scale(scale);
        let output = renderer
            .create_empty_texture(NonZeroU32::new(256).unwrap(), NonZeroU32::new(256).unwrap())
            .unwrap();
        drop(renderer.apply_filter(
            input.clone(),
            (16, 8),
            (224, 240),
            output.clone(),
            (5, 3),
            glow.clone(),
        ));
        let mut screen = CommandList::new();
        screen.render_bitmap(
            output,
            Transform::default(),
            false,
            PixelSnapping::Never,
            PixelRegion::for_whole_size(256, 256),
        );
        renderer.submit_frame(Color::BLACK, screen, vec![]);
        outputs.push(renderer.capture_frame().unwrap());
    }
    assert_eq!(
        outputs[0], outputs[1],
        "Display blur setting changed BitmapData.applyFilter pixels"
    );
    println!("Sharp source pixels and exact BitmapData filters passed");
}

#[cfg(target_family = "wasm")]
fn main() {}
