//! Walks a content stream and decides, operator by operator, what stays.
//!
//! The interpreter follows exactly the state that decides *where* something is
//! drawn — the transformation matrix, the text state and matrices, the font —
//! and nothing that only decides how it looks. For each text-showing operator
//! it lays the glyphs out and takes out every glyph that lies in a marked
//! area; the operator is written again as a `TJ` in which each removed glyph
//! is replaced by a number equal to its advance, so every glyph after it
//! stays where it was (§9.4.3). Images, form XObjects and paths go through
//! their own rules; everything else is copied byte for byte.
//!
//! **When is a glyph "in" an area?** When at least a quarter of its box is
//! inside, or its centre is. The box is the advance by the font's ascent and
//! descent. The quarter keeps a line whose descenders dip a point into the
//! area of the line below from being taken out with it; the centre catches a
//! narrow glyph under a narrow area. A glyph with no size at all — font size
//! zero, used for invisible text — is judged by its origin.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::content::{self, Op};
use crate::error::{RedactError, Result};
use crate::file::{Pdf, Stored, Stream};
use crate::filters::{decode_plain, deflate};
use crate::font::{Font, FontCache};
use crate::geom::{bounds, centre, covered_fraction, polygon_area, quad, Matrix, Quad, Rect};
use crate::image::{self, ImageFate};
use crate::object::{fmt_real, to_bytes, write_name, write_string, Dict, Obj, Ref};

/// Fraction of a glyph's box that puts it inside an area.
pub const GLYPH_COVER: f64 = 0.25;

/// Form XObjects nest at most this deep; deeper is a loop or hostile.
const MAX_FORM_DEPTH: usize = 12;

/// Counts of what was taken out, for the report the user sees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub glyphs: u32,
    pub images_removed: u32,
    pub images_cleared: u32,
    /// Images removed whole because their format cannot be edited.
    pub images_unsupported: u32,
    pub paths: u32,
    pub forms: u32,
    pub marked_content: u32,
}

impl Counts {
    pub fn add(&mut self, o: &Counts) {
        self.glyphs += o.glyphs;
        self.images_removed += o.images_removed;
        self.images_cleared += o.images_cleared;
        self.images_unsupported += o.images_unsupported;
        self.paths += o.paths;
        self.forms += o.forms;
        self.marked_content += o.marked_content;
    }

    pub fn anything(&self) -> bool {
        *self != Counts::default()
    }
}

/// Shared across one page and the forms it draws.
#[derive(Debug)]
pub struct Env<'r, 'a> {
    pub pdf: &'r mut Pdf<'a>,
    pub fonts: FontCache,
    pub areas: Vec<Rect>,
    pub counts: Counts,
    forms_open: Vec<u32>,
    names: u32,
}

impl<'r, 'a> Env<'r, 'a> {
    pub fn new(pdf: &'r mut Pdf<'a>, areas: Vec<Rect>) -> Self {
        Env {
            pdf,
            fonts: FontCache::default(),
            areas,
            counts: Counts::default(),
            forms_open: Vec::new(),
            names: 0,
        }
    }
}

/// The result of one stream.
#[derive(Debug, Default)]
pub struct Outcome {
    /// The rewritten stream, or `None` when nothing in it changed.
    pub content: Option<Vec<u8>>,
    /// XObjects the rewritten stream draws under new names.
    pub additions: Vec<(Vec<u8>, Ref)>,
    /// Marked-content ids (`/MCID`) whose content lost something.
    pub touched_mcids: Vec<i64>,
    /// XObject names no longer drawn by the rewritten stream: they came out
    /// of its resources, so the original object is not kept alive by them.
    pub removed_names: Vec<Vec<u8>>,
    /// XObject names the stream still draws. Reported whether or not the
    /// stream changed, because a form that inherits its parent's resources
    /// draws through the parent's names.
    pub used_names: BTreeSet<Vec<u8>>,
    /// The original XObjects this stream stopped drawing.
    pub replaced: Vec<Ref>,
}

#[derive(Debug, Clone)]
struct GState {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    line_width: f64,
}

impl GState {
    fn new(ctm: Matrix) -> Self {
        GState {
            ctm,
            font: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            line_width: 1.0,
        }
    }
}

#[derive(Debug)]
struct Marked {
    op: usize,
    tag: Vec<u8>,
    props: Option<Dict>,
    touched: bool,
}

#[derive(Debug)]
enum Edit {
    Drop,
    Replace(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Str(Vec<u8>),
    Num(f64),
}

struct Run<'e, 'r, 'a> {
    env: &'e mut Env<'r, 'a>,
    resources: Dict,
    g: GState,
    stack: Vec<GState>,
    tm: Matrix,
    tlm: Matrix,
    marked: Vec<Marked>,
    edits: Vec<Option<Edit>>,
    path_ops: Vec<usize>,
    path_pts: Vec<(f64, f64)>,
    clipping: bool,
    additions: Vec<(Vec<u8>, Ref)>,
    touched_mcids: Vec<i64>,
    /// Names of `Do` operators taken out or redirected, with what they drew.
    redirected: Vec<(Vec<u8>, Option<Ref>)>,
    used_names: BTreeSet<Vec<u8>>,
    nested_replaced: Vec<Ref>,
    changed: bool,
    depth: usize,
}

/// Interprets `content` drawn with `resources` under `ctm`.
pub fn run(
    env: &mut Env<'_, '_>,
    content: &[u8],
    resources: &Dict,
    ctm: Matrix,
    depth: usize,
) -> Result<Outcome> {
    let comps = |cs: &Obj| image::components(None, cs, None);
    // Named inline colour spaces need the resources; resolve through them.
    let pdf_ref: &Pdf<'_> = env.pdf;
    let res_for_cs = resources.clone();
    let comps_res =
        |cs: &Obj| image::components(Some(pdf_ref), cs, Some(&res_for_cs)).or_else(|| comps(cs));
    let ops = content::parse(content, &comps_res)?;
    let mut run = Run {
        env,
        resources: resources.clone(),
        g: GState::new(ctm),
        stack: Vec::new(),
        tm: Matrix::IDENTITY,
        tlm: Matrix::IDENTITY,
        marked: Vec::new(),
        edits: (0..ops.len()).map(|_| None).collect(),
        path_ops: Vec::new(),
        path_pts: Vec::new(),
        clipping: false,
        additions: Vec::new(),
        touched_mcids: Vec::new(),
        redirected: Vec::new(),
        used_names: BTreeSet::new(),
        nested_replaced: Vec::new(),
        changed: false,
        depth,
    };
    for (i, op) in ops.iter().enumerate() {
        run.step(i, op)?;
    }
    // Marked content left open at the end of the stream closes there.
    while let Some(m) = run.marked.pop() {
        run.close_marked(m)?;
    }
    let used_names = std::mem::take(&mut run.used_names);
    if !run.changed {
        return Ok(Outcome {
            used_names,
            ..Outcome::default()
        });
    }
    let removed_names: Vec<Vec<u8>> = run
        .redirected
        .iter()
        .filter(|(n, _)| !used_names.contains(n))
        .map(|(n, _)| n.clone())
        .collect();
    let mut replaced: Vec<Ref> = run.redirected.iter().filter_map(|(_, r)| *r).collect();
    replaced.extend(run.nested_replaced.iter().copied());
    let mut out = Vec::with_capacity(content.len());
    for (op, edit) in ops.iter().zip(&run.edits) {
        match edit {
            None => out.extend_from_slice(content.get(op.start..op.end).unwrap_or_default()),
            Some(Edit::Drop) => continue,
            Some(Edit::Replace(b)) => out.extend_from_slice(b),
        }
        out.push(b'\n');
    }
    Ok(Outcome {
        content: Some(out),
        additions: run.additions,
        touched_mcids: run.touched_mcids,
        removed_names,
        used_names,
        replaced,
    })
}

impl Run<'_, '_, '_> {
    fn edit(&mut self, i: usize, e: Edit) {
        if let Some(slot) = self.edits.get_mut(i) {
            *slot = Some(e);
        }
        self.changed = true;
        for m in &mut self.marked {
            m.touched = true;
        }
    }

    /// A resource by category and name, resolved.
    fn resource(&self, category: &[u8], name: &[u8]) -> Result<Option<Obj>> {
        let Some(sub) = self.resources.get(category) else {
            return Ok(None);
        };
        let Some(sub) = self.env.pdf.resolve_dict(sub)? else {
            return Ok(None);
        };
        Ok(sub.get(name).cloned())
    }

    fn hits(&self, q: &Quad, origin: (f64, f64)) -> bool {
        let degenerate = polygon_area(q) < 1e-9;
        let (cx, cy) = centre(q);
        self.env.areas.iter().any(|a| {
            if degenerate {
                a.contains(origin.0, origin.1)
            } else {
                a.contains(cx, cy) || covered_fraction(q, a) >= GLYPH_COVER
            }
        })
    }

    fn step(&mut self, i: usize, op: &Op) -> Result<()> {
        match op.operator.as_slice() {
            b"q" => self.stack.push(self.g.clone()),
            b"Q" => {
                if let Some(g) = self.stack.pop() {
                    self.g = g;
                }
            }
            b"cm" => {
                let m = Matrix::new(
                    op.num(0),
                    op.num(1),
                    op.num(2),
                    op.num(3),
                    op.num(4),
                    op.num(5),
                );
                self.g.ctm = m.then(&self.g.ctm);
            }
            b"w" => self.g.line_width = op.num(0),
            b"gs" => self.ext_gstate(op)?,
            b"BT" => {
                self.tm = Matrix::IDENTITY;
                self.tlm = Matrix::IDENTITY;
            }
            b"Tc" => self.g.char_spacing = op.num(0),
            b"Tw" => self.g.word_spacing = op.num(0),
            b"Tz" => self.g.scale = op.num(0) / 100.0,
            b"TL" => self.g.leading = op.num(0),
            b"Ts" => self.g.rise = op.num(0),
            b"Tf" => {
                let name = op
                    .operands
                    .first()
                    .and_then(Obj::as_name)
                    .unwrap_or_default()
                    .to_vec();
                self.g.size = op.num(1);
                self.g.font = match self.resource(b"Font", &name)? {
                    Some(f) => Some(self.env.fonts.get(self.env.pdf, &f)?),
                    None => None,
                };
            }
            b"Td" => self.move_line(op.num(0), op.num(1)),
            b"TD" => {
                self.g.leading = -op.num(1);
                self.move_line(op.num(0), op.num(1));
            }
            b"Tm" => {
                self.tlm = Matrix::new(
                    op.num(0),
                    op.num(1),
                    op.num(2),
                    op.num(3),
                    op.num(4),
                    op.num(5),
                );
                self.tm = self.tlm;
            }
            b"T*" => self.move_line(0.0, -self.g.leading),
            b"Tj" => self.show(
                i,
                op.operands.first().cloned().into_iter().collect(),
                Vec::new(),
            )?,
            b"TJ" => {
                let items = op
                    .operands
                    .first()
                    .and_then(Obj::as_array)
                    .unwrap_or_default()
                    .to_vec();
                self.show(i, items, Vec::new())?;
            }
            b"'" => {
                self.move_line(0.0, -self.g.leading);
                self.show(
                    i,
                    op.operands.first().cloned().into_iter().collect(),
                    b"T*\n".to_vec(),
                )?;
            }
            b"\"" => {
                self.g.word_spacing = op.num(0);
                self.g.char_spacing = op.num(1);
                self.move_line(0.0, -self.g.leading);
                let prefix = format!("{} Tw {} Tc T*\n", fmt_real(op.num(0)), fmt_real(op.num(1)));
                self.show(
                    i,
                    op.operands.get(2).cloned().into_iter().collect(),
                    prefix.into_bytes(),
                )?;
            }
            b"m" | b"l" => {
                self.path_ops.push(i);
                let p = self.g.ctm.apply(op.num(0), op.num(1));
                self.path_pts.push(p);
            }
            b"c" => {
                self.path_ops.push(i);
                for k in 0..3 {
                    let p = self.g.ctm.apply(op.num(2 * k), op.num(2 * k + 1));
                    self.path_pts.push(p);
                }
            }
            b"v" | b"y" => {
                self.path_ops.push(i);
                for k in 0..2 {
                    let p = self.g.ctm.apply(op.num(2 * k), op.num(2 * k + 1));
                    self.path_pts.push(p);
                }
            }
            b"re" => {
                self.path_ops.push(i);
                let r = Rect::new(
                    op.num(0),
                    op.num(1),
                    op.num(0) + op.num(2),
                    op.num(1) + op.num(3),
                );
                self.path_pts.extend(quad(&r, &self.g.ctm));
            }
            b"h" => self.path_ops.push(i),
            b"W" | b"W*" => {
                self.path_ops.push(i);
                self.clipping = true;
            }
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                self.paint(i, op)
            }
            b"Do" => self.draw_xobject(i, op)?,
            b"BI" => self.inline_image(i, op)?,
            b"BDC" | b"BMC" => {
                let props = match op.operands.get(1) {
                    Some(Obj::Dict(d)) => Some(d.clone()),
                    Some(Obj::Name(n)) => match self.resource(b"Properties", n)? {
                        Some(p) => self.env.pdf.resolve_dict(&p)?,
                        None => None,
                    },
                    _ => None,
                };
                let tag = op
                    .operands
                    .first()
                    .and_then(Obj::as_name)
                    .unwrap_or_default()
                    .to_vec();
                self.marked.push(Marked {
                    op: i,
                    tag,
                    props,
                    touched: false,
                });
            }
            b"EMC" => {
                if let Some(m) = self.marked.pop() {
                    self.close_marked(m)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn ext_gstate(&mut self, op: &Op) -> Result<()> {
        let Some(name) = op.operands.first().and_then(Obj::as_name) else {
            return Ok(());
        };
        let Some(gs) = self.resource(b"ExtGState", name)? else {
            return Ok(());
        };
        let Some(gs) = self.env.pdf.resolve_dict(&gs)? else {
            return Ok(());
        };
        if let Some(lw) = gs.get(b"LW").and_then(Obj::as_f64) {
            self.g.line_width = lw;
        }
        if let Some(Obj::Array(f)) = gs
            .get(b"Font")
            .map(|f| self.env.pdf.resolve(f))
            .transpose()?
        {
            if let (Some(font), Some(size)) = (f.first(), f.get(1).and_then(Obj::as_f64)) {
                self.g.font = Some(self.env.fonts.get(self.env.pdf, font)?);
                self.g.size = size;
            }
        }
        Ok(())
    }

    fn move_line(&mut self, tx: f64, ty: f64) {
        self.tlm = Matrix::translate(tx, ty).then(&self.tlm);
        self.tm = self.tlm;
    }

    /// Lays out the glyphs of a showing operator, removing those in an area.
    fn show(&mut self, i: usize, items: Vec<Obj>, prefix: Vec<u8>) -> Result<()> {
        let has_text = items
            .iter()
            .any(|o| matches!(o, Obj::Str(s) if !s.is_empty()));
        let Some(font) = self.g.font.clone() else {
            if has_text {
                return Err(RedactError::Unsupported(
                    "teks ditampilkan tanpa font".into(),
                ));
            }
            return Ok(());
        };
        let size = self.g.size;
        let th = self.g.scale;
        let mut out: Vec<Item> = Vec::new();
        let mut removed = 0u32;
        for item in &items {
            match item {
                Obj::Str(s) => {
                    let mut kept: Vec<u8> = Vec::new();
                    for gl in font.split(s) {
                        let word = if font.is_word_space(&gl) {
                            self.g.word_spacing
                        } else {
                            0.0
                        };
                        // Text space to page: size and scale, rise, the text
                        // matrix, then the CTM (§9.4.4).
                        let trm = Matrix::new(size * th, 0.0, 0.0, size, 0.0, self.g.rise)
                            .then(&self.tm)
                            .then(&self.g.ctm);
                        let glyph_to_page = font.matrix.then(&trm);
                        let w = font.width(gl.code);
                        let (bx, advance) = if font.vertical {
                            let (w1, vx, vy) = font.vertical_metrics(gl.code);
                            let bx = Rect::new(-vx, font.descent - vy, w - vx, font.ascent - vy);
                            let adv = w1 * font.matrix.d * size + self.g.char_spacing + word;
                            (bx, adv)
                        } else {
                            let bx = Rect::new(0.0, font.descent, w, font.ascent);
                            let w0 = w * font.matrix.a;
                            let adv = (w0 * size + self.g.char_spacing + word) * th;
                            (bx, adv)
                        };
                        let q = quad(&bx, &glyph_to_page);
                        let origin = trm.apply(0.0, 0.0);
                        let gone = self.hits(&q, origin);
                        let bytes = s.get(gl.start..gl.start + gl.len).unwrap_or_default();
                        if gone {
                            removed += 1;
                            if !kept.is_empty() {
                                out.push(Item::Str(std::mem::take(&mut kept)));
                            }
                            // The number that moves the pen as far as the
                            // glyph would have: tx = -n/1000 · size · scale.
                            let displacement = if font.vertical {
                                advance
                            } else {
                                advance / th.max(1e-12)
                            };
                            if size.abs() < 1e-12 {
                                if displacement.abs() > 1e-9 {
                                    return Err(RedactError::Unsupported(
                                        "glyph berukuran nol dengan spasi karakter".into(),
                                    ));
                                }
                            } else {
                                out.push(Item::Num(-displacement / size * 1000.0));
                            }
                        } else {
                            kept.extend_from_slice(bytes);
                        }
                        self.tm = if font.vertical {
                            Matrix::translate(0.0, advance).then(&self.tm)
                        } else {
                            Matrix::translate(advance, 0.0).then(&self.tm)
                        };
                    }
                    if !kept.is_empty() {
                        out.push(Item::Str(kept));
                    }
                }
                other => {
                    let n = other.as_f64().unwrap_or(0.0);
                    let shift = -n / 1000.0 * size;
                    self.tm = if font.vertical {
                        Matrix::translate(0.0, shift).then(&self.tm)
                    } else {
                        Matrix::translate(shift * th, 0.0).then(&self.tm)
                    };
                    out.push(Item::Num(n));
                }
            }
        }
        if removed == 0 {
            return Ok(());
        }
        self.env.counts.glyphs += removed;
        let mut bytes = prefix;
        bytes.push(b'[');
        let mut pending: Option<f64> = None;
        let mut first = true;
        let sep = |b: &mut Vec<u8>, first: &mut bool| {
            if !*first {
                b.push(b' ');
            }
            *first = false;
        };
        for item in out {
            match item {
                Item::Num(n) => *pending.get_or_insert(0.0) += n,
                Item::Str(s) => {
                    if let Some(n) = pending.take() {
                        sep(&mut bytes, &mut first);
                        bytes.extend_from_slice(fmt_real(n).as_bytes());
                    }
                    sep(&mut bytes, &mut first);
                    write_string(&mut bytes, &s);
                }
            }
        }
        if let Some(n) = pending {
            sep(&mut bytes, &mut first);
            bytes.extend_from_slice(fmt_real(n).as_bytes());
        }
        bytes.extend_from_slice(b"] TJ");
        self.edit(i, Edit::Replace(bytes));
        Ok(())
    }

    /// A path is painted: remove it when it lies wholly inside one area.
    /// Clipping paths are never removed — taking one away would show what it
    /// hid, which may be outside every area.
    fn paint(&mut self, i: usize, op: &Op) {
        let ops = std::mem::take(&mut self.path_ops);
        let pts = std::mem::take(&mut self.path_pts);
        let clipping = std::mem::replace(&mut self.clipping, false);
        if clipping || op.is(b"n") {
            return;
        }
        let Some(b) = bounds(&pts) else { return };
        let stroked = matches!(
            op.operator.as_slice(),
            b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*"
        );
        let b = if stroked {
            b.inflate(self.g.line_width.max(0.0) * self.g.ctm.max_scale())
        } else {
            b
        };
        if self.env.areas.iter().any(|a| a.contains_rect(&b)) {
            for k in ops {
                self.edit(k, Edit::Drop);
            }
            self.edit(i, Edit::Drop);
            self.env.counts.paths += 1;
        }
    }

    fn fresh_name(&mut self, existing: Option<&Dict>) -> Vec<u8> {
        loop {
            self.env.names += 1;
            let n = format!("IzRd{}", self.env.names).into_bytes();
            let clash = existing.is_some_and(|d| d.contains(&n))
                || self.additions.iter().any(|(a, _)| *a == n);
            if !clash {
                return n;
            }
        }
    }

    fn xobject_dict(&self) -> Result<Option<Dict>> {
        match self.resources.get(b"XObject") {
            Some(x) => self.env.pdf.resolve_dict(x),
            None => Ok(None),
        }
    }

    fn register(&mut self, stored: Stored) -> Result<Vec<u8>> {
        let r = self.env.pdf.add(stored);
        let existing = self.xobject_dict()?;
        let name = self.fresh_name(existing.as_ref());
        self.additions.push((name.clone(), r));
        Ok(name)
    }

    fn do_bytes(name: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        write_name(&mut b, name);
        b.extend_from_slice(b" Do");
        b
    }

    fn draw_xobject(&mut self, i: usize, op: &Op) -> Result<()> {
        let Some(name) = op.operands.first().and_then(Obj::as_name) else {
            return Ok(());
        };
        let name = name.to_vec();
        let r = self.resource(b"XObject", &name)?;
        if let Some(r) = &r {
            self.draw_xobject_ref(i, r)?;
        }
        if self.edits.get(i).is_some_and(Option::is_some) {
            self.redirected
                .push((name, r.as_ref().and_then(Obj::as_ref)));
        } else {
            self.used_names.insert(name);
        }
        Ok(())
    }

    fn draw_xobject_ref(&mut self, i: usize, r: &Obj) -> Result<()> {
        let r = r.clone();
        let Some(stream) = self.env.pdf.stream_of(&r)? else {
            return Ok(());
        };
        match stream.dict.name(b"Subtype") {
            Some(b"Image") => {
                let fate = image::redact(
                    Some(self.env.pdf),
                    &stream.dict,
                    &stream.data,
                    &self.g.ctm,
                    &self.env.areas,
                    Some(&self.resources),
                    false,
                )?;
                match fate {
                    ImageFate::Keep => {}
                    ImageFate::Remove { unsupported } => {
                        if unsupported {
                            self.env.counts.images_unsupported += 1;
                        } else {
                            self.env.counts.images_removed += 1;
                        }
                        self.edit(i, Edit::Drop);
                    }
                    ImageFate::Replace {
                        mut dict,
                        data,
                        smask,
                        mask,
                    } => {
                        if let Some(s) = smask {
                            let r = self.env.pdf.add(Stored::Stream(s));
                            dict.set(b"SMask", Obj::Ref(r));
                        }
                        if let Some(m) = mask {
                            let r = self.env.pdf.add(Stored::Stream(m));
                            dict.set(b"Mask", Obj::Ref(r));
                        }
                        let new = self.register(Stored::Stream(Stream { dict, data }))?;
                        self.env.counts.images_cleared += 1;
                        self.edit(i, Edit::Replace(Self::do_bytes(&new)));
                    }
                }
            }
            Some(b"Form") => self.draw_form(i, &r, stream)?,
            _ => {}
        }
        Ok(())
    }

    fn draw_form(&mut self, i: usize, r: &Obj, stream: Stream) -> Result<()> {
        let num = r.as_ref().map(|r| r.num);
        if self.depth >= MAX_FORM_DEPTH || num.is_some_and(|n| self.env.forms_open.contains(&n)) {
            return Ok(());
        }
        let matrix = match stream
            .dict
            .get(b"Matrix")
            .map(|m| self.env.pdf.resolve(m))
            .transpose()?
        {
            Some(Obj::Array(m)) if m.len() == 6 => {
                let v: Vec<f64> = m.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
                Matrix::new(
                    v.first().copied().unwrap_or(1.0),
                    v.get(1).copied().unwrap_or(0.0),
                    v.get(2).copied().unwrap_or(0.0),
                    v.get(3).copied().unwrap_or(1.0),
                    v.get(4).copied().unwrap_or(0.0),
                    v.get(5).copied().unwrap_or(0.0),
                )
            }
            _ => Matrix::IDENTITY,
        };
        let resources = match stream.dict.get(b"Resources") {
            Some(res) => self.env.pdf.resolve_dict(res)?.unwrap_or_default(),
            // A form without resources uses its parent's (§7.8.3, PDF 1.1).
            None => self.resources.clone(),
        };
        let content = decode_plain(Some(self.env.pdf), &stream.dict, &stream.data)?;
        let ctm = matrix.then(&self.g.ctm);
        if let Some(n) = num {
            self.env.forms_open.push(n);
        }
        let outcome = run(self.env, &content, &resources, ctm, self.depth + 1);
        if num.is_some() {
            self.env.forms_open.pop();
        }
        let outcome = outcome?;
        if stream.dict.get(b"Resources").is_none() {
            self.used_names.extend(outcome.used_names.iter().cloned());
        }
        self.nested_replaced
            .extend(outcome.replaced.iter().copied());
        let Some(new_content) = outcome.content else {
            return Ok(());
        };
        let mut dict = stream.dict.clone();
        for k in [
            &b"Length"[..],
            b"Filter",
            b"DecodeParms",
            b"PieceInfo",
            b"Metadata",
            b"LastModified",
        ] {
            dict.remove(k);
        }
        dict.set(b"Filter", Obj::name("FlateDecode"));
        if !outcome.additions.is_empty()
            || !outcome.removed_names.is_empty()
            || stream.dict.get(b"Resources").is_none()
        {
            dict.set(
                b"Resources",
                Obj::Dict(with_xobjects(
                    self.env.pdf,
                    &resources,
                    &outcome.additions,
                    &outcome.removed_names,
                )?),
            );
        }
        let new = self.register(Stored::Stream(Stream {
            dict,
            data: deflate(&new_content),
        }))?;
        self.env.counts.forms += 1;
        self.edit(i, Edit::Replace(Self::do_bytes(&new)));
        Ok(())
    }

    fn inline_image(&mut self, i: usize, op: &Op) -> Result<()> {
        let Some(img) = &op.inline else { return Ok(()) };
        let fate = image::redact(
            Some(self.env.pdf),
            &img.dict,
            &img.data,
            &self.g.ctm,
            &self.env.areas,
            Some(&self.resources),
            true,
        )?;
        match fate {
            ImageFate::Keep => {}
            ImageFate::Remove { unsupported } => {
                if unsupported {
                    self.env.counts.images_unsupported += 1;
                } else {
                    self.env.counts.images_removed += 1;
                }
                self.edit(i, Edit::Drop);
            }
            ImageFate::Replace { dict, data, .. } => {
                let mut b = b"BI".to_vec();
                for (k, v) in &dict.0 {
                    b.push(b' ');
                    write_name(&mut b, k);
                    b.push(b' ');
                    b.extend_from_slice(&to_bytes(v));
                }
                b.extend_from_slice(b" ID ");
                b.extend_from_slice(&data);
                b.extend_from_slice(b"\nEI");
                self.env.counts.images_cleared += 1;
                self.edit(i, Edit::Replace(b));
            }
        }
        Ok(())
    }

    /// At `EMC`: marked content whose content lost something loses its
    /// replacement text too (`/ActualText`, `/Alt`, `/E`), which would
    /// otherwise hand the removed words to any extractor that honours it.
    fn close_marked(&mut self, m: Marked) -> Result<()> {
        if !m.touched {
            return Ok(());
        }
        // The enclosing sequences contain this one, so they were touched too.
        for outer in &mut self.marked {
            outer.touched = true;
        }
        let Some(props) = m.props else { return Ok(()) };
        if let Some(mcid) = props.get(b"MCID").and_then(Obj::as_i64) {
            self.touched_mcids.push(mcid);
        }
        let leaks = [&b"ActualText"[..], b"Alt", b"E"];
        if !leaks.iter().any(|k| props.contains(k)) {
            return Ok(());
        }
        let mut clean = props.clone();
        for k in leaks {
            clean.remove(k);
        }
        let mut b = Vec::new();
        write_name(&mut b, &m.tag);
        b.push(b' ');
        b.extend_from_slice(&to_bytes(&Obj::Dict(clean)));
        b.extend_from_slice(b" BDC");
        self.env.counts.marked_content += 1;
        self.edit(m.op, Edit::Replace(b));
        Ok(())
    }
}

/// `resources` with `additions` added to its `/XObject` subdictionary, as a
/// direct dictionary (shared subdictionaries are copied, never changed in
/// place — another page may be using them).
pub fn with_xobjects(
    pdf: &Pdf<'_>,
    resources: &Dict,
    additions: &[(Vec<u8>, Ref)],
    removed: &[Vec<u8>],
) -> Result<Dict> {
    let mut res = resources.clone();
    if additions.is_empty() && removed.is_empty() {
        return Ok(res);
    }
    let mut xobjects = match res.get(b"XObject") {
        Some(x) => pdf.resolve_dict(x)?.unwrap_or_default(),
        None => Dict::new(),
    };
    for name in removed {
        xobjects.remove(name);
    }
    for (name, r) in additions {
        xobjects.set(name, Obj::Ref(*r));
    }
    res.set(b"XObject", Obj::Dict(xobjects));
    Ok(res)
}
