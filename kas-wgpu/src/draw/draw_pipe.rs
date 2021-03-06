// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License in the LICENSE-APACHE file or at:
//     https://www.apache.org/licenses/LICENSE-2.0

//! Drawing API for `kas_wgpu`
//!
//! TODO: move traits up to kas?

use std::any::Any;
use std::f32::consts::FRAC_PI_2;
use wgpu_glyph::GlyphBrushBuilder;

use super::{Colour, Draw, DrawPipe, FlatRound, ShadedRound, ShadedSquare, Vec2};
use crate::shared::SharedState;
use kas::geom::{Coord, Rect, Size};
use kas::theme;

/// Style of drawing
pub enum ShadeStyle {
    /// Square corners, shading according to the given normals
    ///
    /// Normal has two components, `(outer, inner)`, interpreted as the
    /// horizontal component of the direction vector outwards from the drawn
    /// feature. Both values are constrained to the closed range `[-1, 1]`.
    Square(Vec2),
    /// Round corners, shading according to the given normals
    ///
    /// Normal has two components, `(outer, inner)`, interpreted as the
    /// horizontal component of the direction vector outwards from the drawn
    /// feature. Both values are constrained to the closed range `[-1, 1]`.
    Round(Vec2),
}

/// Abstraction over drawing commands specific to `kas_wgpu`
pub trait DrawExt: Draw {
    /// Add a rounded flat frame to the draw buffer.
    fn rounded_frame(&mut self, region: Self::Region, outer: Rect, inner: Rect, col: Colour);

    /// Add a rounded shaded frame to the draw buffer.
    fn shaded_frame(
        &mut self,
        region: Self::Region,
        outer: Rect,
        inner: Rect,
        style: ShadeStyle,
        col: Colour,
    );
}

impl DrawPipe {
    /// Construct
    // TODO: do we want to share state across windows? With glyph_brush this is
    // not trivial but with our "pipes" it shouldn't be difficult.
    pub fn new<T: theme::Theme<Self>>(
        shared: &mut SharedState<T>,
        tex_format: wgpu::TextureFormat,
        size: Size,
    ) -> Self {
        let dir = shared.theme.light_direction();
        assert!(dir.0 >= 0.0);
        assert!(dir.0 < FRAC_PI_2);
        let a = (dir.0.sin(), dir.0.cos());
        // We normalise intensity:
        let f = a.0 / a.1;
        let norm = [dir.1.sin() * f, -dir.1.cos() * f, 1.0];

        let glyph_brush = GlyphBrushBuilder::using_fonts(shared.theme.get_fonts())
            .build(&mut shared.device, tex_format);

        let region = Rect {
            pos: Coord::ZERO,
            size,
        };
        DrawPipe {
            clip_regions: vec![region],
            flat_round: FlatRound::new(shared, size),
            shaded_square: ShadedSquare::new(shared, size, norm),
            shaded_round: ShadedRound::new(shared, size, norm),
            glyph_brush,
        }
    }

    /// Process window resize
    pub fn resize(&mut self, device: &wgpu::Device, size: Size) -> wgpu::CommandBuffer {
        self.clip_regions[0].size = size;
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { todo: 0 });
        self.flat_round.resize(device, &mut encoder, size);
        self.shaded_square.resize(device, &mut encoder, size);
        self.shaded_round.resize(device, &mut encoder, size);
        encoder.finish()
    }

    /// Render batched draw instructions via `rpass`
    pub fn render(
        &mut self,
        device: &mut wgpu::Device,
        frame_view: &wgpu::TextureView,
        clear_color: wgpu::Color,
    ) -> wgpu::CommandBuffer {
        let desc = wgpu::CommandEncoderDescriptor { todo: 0 };
        let mut encoder = device.create_command_encoder(&desc);
        let mut load_op = wgpu::LoadOp::Clear;

        // We use a separate render pass for each clipped region.
        for (pass, region) in self.clip_regions.iter().enumerate() {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment: frame_view,
                    resolve_target: None,
                    load_op: load_op,
                    store_op: wgpu::StoreOp::Store,
                    clear_color,
                }],
                depth_stencil_attachment: None,
            });
            rpass.set_scissor_rect(
                region.pos.0 as u32,
                region.pos.1 as u32,
                region.size.0,
                region.size.1,
            );

            self.flat_round.render(device, pass, &mut rpass);
            self.shaded_square.render(device, pass, &mut rpass);
            self.shaded_round.render(device, pass, &mut rpass);
            drop(rpass);

            load_op = wgpu::LoadOp::Load;
        }

        // Fonts use their own render pass(es).
        let size = self.clip_regions[0].size;
        self.glyph_brush
            .draw_queued(device, &mut encoder, frame_view, size.0, size.1)
            .expect("glyph_brush.draw_queued");

        // Keep only first clip region (which is the entire window)
        self.clip_regions.truncate(1);

        encoder.finish()
    }
}

impl Draw for DrawPipe {
    type Region = usize;

    #[inline]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn add_clip_region(&mut self, region: Rect) -> usize {
        let pass = self.clip_regions.len();
        self.clip_regions.push(region);
        pass
    }

    #[inline]
    fn rect(&mut self, region: Self::Region, rect: Rect, col: Colour) {
        self.shaded_square.rect(region, rect, col);
    }

    #[inline]
    fn frame(&mut self, region: Self::Region, outer: Rect, inner: Rect, col: Colour) {
        self.shaded_square.frame(region, outer, inner, col);
    }
}

impl DrawExt for DrawPipe {
    #[inline]
    fn rounded_frame(&mut self, pass: usize, outer: Rect, inner: Rect, col: Colour) {
        self.flat_round.rounded_frame(pass, outer, inner, col);
    }

    #[inline]
    fn shaded_frame(
        &mut self,
        pass: usize,
        outer: Rect,
        inner: Rect,
        style: ShadeStyle,
        col: Colour,
    ) {
        match style {
            ShadeStyle::Square(norm) => self
                .shaded_square
                .shaded_frame(pass, outer, inner, norm, col),
            ShadeStyle::Round(norm) => self
                .shaded_round
                .shaded_frame(pass, outer, inner, norm, col),
        }
    }
}
