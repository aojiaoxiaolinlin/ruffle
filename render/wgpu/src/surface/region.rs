//! Conservative bounds for independent 2D blend layers. Unknown commands use
//! the original full surface; never infer display-object bounds from caches.
use crate::diagnostics::BlendFallback;
use crate::mesh::as_mesh;
use ruffle_render::commands::{Command, CommandList, RenderBlendMode};
use swf::Twips;

#[derive(Clone, Copy, Debug)]
pub struct BlendRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("RUFFLE_GPU_FULL_BLEND_TARGETS").is_none());
    *ENABLED
}

/// Preserve isolation and ancestor-layer semantics; only crop independently bounded 2D content.
pub fn for_blend(
    commands: &CommandList,
    mode: &RenderBlendMode,
    width: u32,
    height: u32,
    active_mask: bool,
) -> Result<BlendRegion, BlendFallback> {
    match mode {
        RenderBlendMode::Shader(_) => Err(BlendFallback::Shader),
        RenderBlendMode::Builtin(swf::BlendMode::Alpha | swf::BlendMode::Erase) => {
            Err(BlendFallback::Ancestor)
        }
        _ if !enabled() => Err(BlendFallback::Disabled),
        // A trivial blend has transparent output outside its own geometry.
        // The parent's stencil still applies when the cropped layer is drawn
        // back, so the parent's active mask does not expand that geometry.
        _ if active_mask
            && !matches!(
                mode,
                RenderBlendMode::Builtin(
                    swf::BlendMode::Normal
                        | swf::BlendMode::Layer
                        | swf::BlendMode::Add
                        | swf::BlendMode::Screen
                        | swf::BlendMode::Subtract
                )
            ) =>
        {
            Err(BlendFallback::ActiveMask)
        }
        _ => region(commands, width, height),
    }
}

impl BlendRegion {
    /// Map the isolated layer back into its parent's pixel coordinates.
    pub fn composite_matrix(self) -> ruffle_render::matrix::Matrix {
        ruffle_render::matrix::Matrix::create_box(
            self.width as f32,
            self.height as f32,
            Twips::from_pixels(f64::from(self.x)),
            Twips::from_pixels(f64::from(self.y)),
        )
    }
}

pub fn region(
    commands: &CommandList,
    width: u32,
    height: u32,
) -> Result<BlendRegion, BlendFallback> {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for command in &commands.commands {
        let (matrix, rect) = match command {
            Command::DrawRect { matrix, .. } => (*matrix, [0.0, 0.0, 1.0, 1.0]),
            Command::RenderBitmap {
                transform,
                region,
                pixel_snapping,
                ..
            } => {
                if transform.tz != 0.0 || transform.perspective_projection.is_some() {
                    return Err(BlendFallback::ThreeD);
                }
                let mut matrix = transform.matrix;
                pixel_snapping.apply(&mut matrix);
                (
                    matrix,
                    [0.0, 0.0, region.width() as f32, region.height() as f32],
                )
            }
            Command::RenderShape { shape, transform } => {
                if transform.tz != 0.0 || transform.perspective_projection.is_some() {
                    return Err(BlendFallback::ThreeD);
                }
                (
                    transform.matrix,
                    as_mesh(shape)
                        .bounds
                        .ok_or(BlendFallback::EmptyOrOffscreen)?,
                )
            }
            // Masks, nested layers, lines, Stage3D and shaders need their own
            // bounds/dependency analysis. Keep their existing path for now.
            Command::Blend(..) => return Err(BlendFallback::Nested),
            Command::RenderAlphaMask { .. }
            | Command::PushMask
            | Command::ActivateMask
            | Command::DeactivateMask
            | Command::PopMask => return Err(BlendFallback::MaskCommands),
            Command::RenderStage3D { .. } => return Err(BlendFallback::ThreeD),
            Command::DrawLine { .. } | Command::DrawLineRect { .. } => {
                return Err(BlendFallback::Lines);
            }
        };
        for (x, y) in [
            (rect[0], rect[1]),
            (rect[2], rect[1]),
            (rect[0], rect[3]),
            (rect[2], rect[3]),
        ] {
            let px = matrix.a * x + matrix.c * y + matrix.tx.to_pixels() as f32;
            let py = matrix.b * x + matrix.d * y + matrix.ty.to_pixels() as f32;
            if !px.is_finite() || !py.is_finite() {
                return Err(BlendFallback::InvalidBounds);
            }
            bounds[0] = bounds[0].min(px);
            bounds[1] = bounds[1].min(py);
            bounds[2] = bounds[2].max(px);
            bounds[3] = bounds[3].max(py);
        }
    }
    // Integer origins preserve pixel snapping and MSAA sample locations. Leave
    // a conservative guard for rasterization at transformed geometry edges.
    // Bucket dimensions to avoid a different pool key for every animation pixel.
    let x = (((bounds[0] - 2.0) / 32.0).floor() * 32.0).clamp(0.0, width as f32) as u32;
    let y = (((bounds[1] - 2.0) / 32.0).floor() * 32.0).clamp(0.0, height as f32) as u32;
    let right = (((bounds[2] + 2.0) / 32.0).ceil() * 32.0).clamp(0.0, width as f32) as u32;
    let bottom = (((bounds[3] + 2.0) / 32.0).ceil() * 32.0).clamp(0.0, height as f32) as u32;
    if right <= x || bottom <= y {
        return Err(BlendFallback::EmptyOrOffscreen);
    }
    if u64::from(right - x) * u64::from(bottom - y) * 4 >= u64::from(width) * u64::from(height) * 3
    {
        return Err(BlendFallback::LargeBounds);
    }
    Ok(BlendRegion {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

pub fn translate(commands: &mut CommandList, region: BlendRegion) {
    for command in &mut commands.commands {
        let matrix = match command {
            Command::DrawRect { matrix, .. } => matrix,
            Command::RenderBitmap { transform, .. } | Command::RenderShape { transform, .. } => {
                &mut transform.matrix
            }
            _ => unreachable!("Only commands accepted by region() can be translated"),
        };
        matrix.tx -= Twips::from_pixels(f64::from(region.x));
        matrix.ty -= Twips::from_pixels(f64::from(region.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruffle_render::commands::CommandHandler;
    use ruffle_render::matrix::Matrix;

    #[test]
    fn trivial_layers_crop_and_restore_nonzero_parent_origin() {
        let mut commands = CommandList::new();
        commands.draw_rect(
            swf::Color::WHITE,
            Matrix::create_box(
                30.0,
                20.0,
                Twips::from_pixels(100.0),
                Twips::from_pixels(50.0),
            ),
        );
        for mode in [
            swf::BlendMode::Normal,
            swf::BlendMode::Layer,
            swf::BlendMode::Add,
            swf::BlendMode::Screen,
            swf::BlendMode::Subtract,
        ] {
            let mode = RenderBlendMode::Builtin(mode);
            let r = for_blend(&commands, &mode, 1000, 700, false).unwrap();
            assert_eq!((r.x, r.y, r.width, r.height), (96, 32, 64, 64));
            let mut local = commands.clone();
            translate(&mut local, r);
            let Command::DrawRect { matrix, .. } = &local.commands[0] else {
                panic!()
            };
            let composite = r.composite_matrix();
            assert_eq!(matrix.tx + composite.tx, Twips::from_pixels(100.0));
            assert_eq!(matrix.ty + composite.ty, Twips::from_pixels(50.0));
            assert_eq!((composite.a, composite.d), (64.0, 64.0));
            let masked = for_blend(&commands, &mode, 1000, 700, true).unwrap();
            assert_eq!(
                (masked.x, masked.y, masked.width, masked.height),
                (96, 32, 64, 64)
            );
        }
    }

    #[test]
    fn layer_with_ancestor_dependent_children_stays_full_size() {
        for child in [swf::BlendMode::Alpha, swf::BlendMode::Erase] {
            let child = RenderBlendMode::Builtin(child);
            assert!(matches!(
                for_blend(&CommandList::new(), &child, 1000, 700, false),
                Err(BlendFallback::Ancestor)
            ));
            let mut commands = CommandList::new();
            commands.blend(CommandList::new(), child);
            assert!(matches!(
                for_blend(
                    &commands,
                    &RenderBlendMode::Builtin(swf::BlendMode::Layer),
                    1000,
                    700,
                    false
                ),
                Err(BlendFallback::Nested)
            ));
        }
    }

    #[test]
    fn complex_blend_inside_active_mask_keeps_full_target() {
        let mut commands = CommandList::new();
        commands.draw_rect(
            swf::Color::WHITE,
            Matrix::create_box(
                30.0,
                20.0,
                Twips::from_pixels(100.0),
                Twips::from_pixels(50.0),
            ),
        );
        assert!(matches!(
            for_blend(
                &commands,
                &RenderBlendMode::Builtin(swf::BlendMode::Multiply),
                1000,
                700,
                true,
            ),
            Err(BlendFallback::ActiveMask)
        ));
    }

    #[test]
    fn fallback_diagnostics_distinguish_dependencies_and_geometry() {
        let mut commands = CommandList::new();
        assert!(matches!(
            region(&commands, 100, 100),
            Err(BlendFallback::EmptyOrOffscreen)
        ));
        commands.draw_rect(swf::Color::WHITE, Matrix::scale(100.0, 100.0));
        assert!(matches!(
            region(&commands, 100, 100),
            Err(BlendFallback::LargeBounds)
        ));
        commands.commands.clear();
        commands.blend(
            CommandList::new(),
            ruffle_render::commands::RenderBlendMode::Builtin(swf::BlendMode::Normal),
        );
        assert!(matches!(
            region(&commands, 100, 100),
            Err(BlendFallback::Nested)
        ));
        commands.commands.clear();
        commands.push_mask();
        assert!(matches!(
            region(&commands, 100, 100),
            Err(BlendFallback::MaskCommands)
        ));
        commands.commands.clear();
        commands.draw_rect(swf::Color::WHITE, Matrix::scale(f32::NAN, 1.0));
        assert!(matches!(
            region(&commands, 100, 100),
            Err(BlendFallback::InvalidBounds)
        ));
    }
    #[test]
    fn transformed_bounds_and_unknown_commands() {
        let mut commands = CommandList::new();
        commands.draw_rect(
            swf::Color::WHITE,
            Matrix::create_box(
                30.0,
                20.0,
                Twips::from_pixels(100.0),
                Twips::from_pixels(50.0),
            ),
        );
        let r = region(&commands, 1000, 700).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (96, 32, 64, 64));
        translate(&mut commands, r);
        if let Command::DrawRect { matrix, .. } = &commands.commands[0] {
            assert_eq!(matrix.tx.to_pixels(), 4.0);
        }
        commands.push_mask();
        assert!(region(&commands, 1000, 700).is_err());
    }
}
