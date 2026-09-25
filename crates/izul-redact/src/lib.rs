//! True redaction (SPEC 17, Phase 6): what lies in a marked area is taken out
//! of the file, not painted over.
//!
//! PDFium has no redaction API, so the worker hands this crate the document
//! PDFium has just written, and gets back a new file in which, on each marked
//! page:
//!
//! * every glyph in an area is gone from the content stream — including
//!   invisible ones (an OCR layer is exactly that) — and the glyphs after it
//!   on the same line are exactly where they were ([`interp`]);
//! * images wholly inside an area are no longer drawn, and the pixels of those
//!   partly inside are cleared ([`image`]);
//! * paths wholly inside an area are gone;
//! * form XObjects are redacted too, as new copies used only by this page;
//! * marked-content replacement text (`/ActualText`, `/Alt`, `/E`) over removed
//!   content is gone, in the content stream and in the structure tree;
//! * annotations over an area are gone, with their popups and — for form
//!   fields — their place in `/AcroForm`;
//! * the page's thumbnail (`/Thumb`) and private application data
//!   (`/PieceInfo`, page `/Metadata`), which can hold a copy of the page as it
//!   was, are gone;
//! * the areas are filled with the chosen colour, as part of the page.
//!
//! The file is written whole ([`file`]), so nothing replaced survives in it.
//!
//! The worker then reopens the result with PDFium and checks that no character
//! is left in an area and that every character outside the areas is where it
//! was. This crate does not trust its own arithmetic; that check is why it
//! does not have to.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod content;
pub mod error;
pub mod file;
pub mod filters;
pub mod font;
pub mod geom;
pub mod image;
pub mod interp;
pub mod object;
pub mod std14;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

pub use error::{RedactError, Result};
pub use geom::{Quad, Rect};
pub use interp::Counts;

use file::{Pdf, Stored, Stream};
use filters::{decode_plain, deflate};
use geom::Matrix;
use interp::{run, with_xobjects, Env};
use object::{fmt_real, Dict, Obj, Ref};

/// One area to redact, in the page's default user space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub rect: Rect,
    /// Colour the area is filled with afterwards (0..1 RGB); `None` leaves it
    /// showing whatever is beneath, which after redaction is the bare page.
    pub fill: Option<[f64; 3]>,
}

/// The areas to redact on one page. All of a page's areas go in one request:
/// the page is interpreted once, against all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct PageAreas {
    /// Index in document order.
    pub page: u32,
    pub areas: Vec<Area>,
}

/// What was taken out of one page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageReport {
    pub page: u32,
    pub counts: Counts,
    pub annotations: u32,
    /// Everywhere outside the areas that something was taken from: the boxes
    /// of glyphs that were mostly — not wholly — inside an area (filled with
    /// that area's colour, since their ink went too), images removed whole
    /// and annotations removed whole. Empty when nothing reached past an area.
    pub beyond: Vec<Quad>,
}

/// Redacts `input` — a file PDFium wrote — and returns the new file.
pub fn redact(input: &[u8], requests: &[PageAreas]) -> Result<(Vec<u8>, Vec<PageReport>)> {
    let mut pdf = Pdf::parse(input)?;
    let pages = pdf.pages()?;
    let mut reports = Vec::new();
    let mut removed_widgets: BTreeSet<u32> = BTreeSet::new();
    let mut gone = Gone::default();
    for req in requests {
        let areas: Vec<Area> = req
            .areas
            .iter()
            .copied()
            .filter(|a| a.rect.area() > 0.0)
            .collect();
        if areas.is_empty() {
            continue;
        }
        let page_ref = *pages
            .get(req.page as usize)
            .ok_or(RedactError::NoPage(req.page))?;
        let report = redact_page(
            &mut pdf,
            page_ref,
            req.page,
            &areas,
            &mut removed_widgets,
            &mut gone,
        )?;
        reports.push(report);
    }
    if !removed_widgets.is_empty() {
        prune_form_fields(&mut pdf, &removed_widgets, &mut gone)?;
    }
    // What was taken out must not stay in the file by another road: a
    // structure tree element pointing at a removed annotation, a calculation
    // order naming a removed field, a resources dictionary another page
    // shares that still lists the original image. Removed annotations and
    // emptied fields become null — every reference to them now reads as
    // nothing. Replaced XObjects become null when nothing draws them any more.
    for num in &gone.objects {
        pdf.set(*num, Stored::Obj(Obj::Null));
    }
    if !gone.replaced.is_empty() {
        let drawn = drawn_everywhere(&pdf)?;
        let other_uses = references_outside_xobject_maps(&pdf)?;
        for r in &gone.replaced {
            if !drawn.contains(&r.num) && !other_uses.contains(&r.num) {
                pdf.set(r.num, Stored::Obj(Obj::Null));
            }
        }
    }
    Ok((pdf.write()?, reports))
}

fn contents_of(pdf: &Pdf<'_>, page: &Dict) -> Result<Vec<u8>> {
    let parts: Vec<Obj> = match page.get(b"Contents") {
        None => Vec::new(),
        Some(c) => match pdf.resolve(c)? {
            Obj::Array(items) => items,
            _ => vec![c.clone()],
        },
    };
    let mut out = Vec::new();
    for part in parts {
        if let Some(s) = pdf.stream_of(&part)? {
            out.extend_from_slice(&decode_plain(Some(pdf), &s.dict, &s.data)?);
            // Streams are concatenated with whitespace between them (§7.8.2):
            // an operator may not span two of them, but its operands and it
            // are allowed to sit on either side of a boundary.
            out.push(b'\n');
        }
    }
    Ok(out)
}

fn rect_of(pdf: &Pdf<'_>, o: Option<&Obj>) -> Result<Option<Rect>> {
    let Some(o) = o else { return Ok(None) };
    let Obj::Array(v) = pdf.resolve(o)? else {
        return Ok(None);
    };
    let n: Vec<f64> = v.iter().filter_map(Obj::as_f64).collect();
    Ok(match n.as_slice() {
        [a, b, c, d] => Some(Rect::new(*a, *b, *c, *d)),
        _ => None,
    })
}

/// Objects the redaction took out, to be erased from the file at the end.
#[derive(Debug, Default)]
struct Gone {
    /// Removed annotations, their popups, and fields left with no widget.
    objects: BTreeSet<u32>,
    /// XObjects a page stopped drawing; erased only if nothing else draws them.
    replaced: Vec<Ref>,
}

#[allow(clippy::too_many_arguments)]
fn redact_page(
    pdf: &mut Pdf<'_>,
    page_ref: Ref,
    index: u32,
    areas: &[Area],
    removed_widgets: &mut BTreeSet<u32>,
    gone: &mut Gone,
) -> Result<PageReport> {
    let rects: Vec<Rect> = areas.iter().map(|a| a.rect).collect();
    let mut page = pdf
        .resolve_dict(&Obj::Ref(page_ref))?
        .ok_or(RedactError::NoPage(index))?;
    let resources = match pdf.inherited(&page, b"Resources")? {
        Some(r) => pdf.resolve_dict(&r)?.unwrap_or_default(),
        None => Dict::new(),
    };
    let content = contents_of(pdf, &page)?;

    let (outcome, counts, spill, vanished) = {
        let mut env = Env::new(pdf, rects.clone());
        let outcome = run(&mut env, &content, &resources, Matrix::IDENTITY, 0)?;
        (outcome, env.counts, env.spill, env.vanished)
    };

    // The page's own content, balanced: whatever state it leaves behind must
    // not reach the fill that follows.
    let mut body = b"q\n".to_vec();
    body.extend_from_slice(outcome.content.as_deref().unwrap_or(&content));
    body.extend_from_slice(b"\nQ\n");
    for area in areas {
        let Some([r, g, b]) = area.fill else { continue };
        let a = area.rect;
        body.extend_from_slice(
            format!(
                "q {} {} {} rg {} {} {} {} re f Q\n",
                fmt_real(r),
                fmt_real(g),
                fmt_real(b),
                fmt_real(a.x0),
                fmt_real(a.y0),
                fmt_real(a.x1 - a.x0),
                fmt_real(a.y1 - a.y0)
            )
            .as_bytes(),
        );
    }
    for (q, area) in &spill {
        let Some([r, g, b]) = areas.get(*area).and_then(|a| a.fill) else {
            continue;
        };
        let [p0, p1, p2, p3] = q;
        // One filled shape per box: sharing a path with the area's rectangle
        // could wind the other way and cut a hole under the nonzero rule.
        body.extend_from_slice(
            format!(
                "q {} {} {} rg {} {} m {} {} l {} {} l {} {} l h f Q\n",
                fmt_real(r),
                fmt_real(g),
                fmt_real(b),
                fmt_real(p0.0),
                fmt_real(p0.1),
                fmt_real(p1.0),
                fmt_real(p1.1),
                fmt_real(p2.0),
                fmt_real(p2.1),
                fmt_real(p3.0),
                fmt_real(p3.1)
            )
            .as_bytes(),
        );
    }
    let mut sdict = Dict::new();
    sdict.set(b"Filter", Obj::name("FlateDecode"));
    let content_ref = pdf.add(Stored::Stream(Stream {
        dict: sdict,
        data: deflate(&body),
    }));
    page.set(b"Contents", Obj::Ref(content_ref));
    if !outcome.additions.is_empty() || !outcome.removed_names.is_empty() {
        page.set(
            b"Resources",
            Obj::Dict(with_xobjects(
                pdf,
                &resources,
                &outcome.additions,
                &outcome.removed_names,
            )?),
        );
    }
    for k in [&b"Thumb"[..], b"PieceInfo", b"Metadata"] {
        page.remove(k);
    }

    gone.replaced.extend(outcome.replaced.iter().copied());
    let mut beyond: Vec<Quad> = spill.iter().map(|(q, _)| *q).collect();
    beyond.extend(vanished);
    let annotations =
        remove_annotations(pdf, &mut page, &rects, removed_widgets, gone, &mut beyond)?;
    if !outcome.touched_mcids.is_empty() {
        strip_structure(pdf, page_ref, &outcome.touched_mcids)?;
    }
    pdf.set(page_ref.num, Stored::Obj(Obj::Dict(page)));
    Ok(PageReport {
        page: index,
        counts,
        annotations,
        beyond,
    })
}

/// Takes out every annotation whose rectangle overlaps an area, and the popups
/// of those taken out.
fn remove_annotations(
    pdf: &Pdf<'_>,
    page: &mut Dict,
    areas: &[Rect],
    removed_widgets: &mut BTreeSet<u32>,
    erased: &mut Gone,
    beyond: &mut Vec<Quad>,
) -> Result<u32> {
    let Some(annots) = page.get(b"Annots").cloned() else {
        return Ok(0);
    };
    let Obj::Array(items) = pdf.resolve(&annots)? else {
        return Ok(0);
    };
    let mut gone: BTreeSet<u32> = BTreeSet::new();
    let mut keep: Vec<(Obj, Dict)> = Vec::new();
    let mut count = 0;
    for item in items {
        let Some(d) = pdf.resolve_dict(&item)? else {
            continue;
        };
        let rect = rect_of(pdf, d.get(b"Rect"))?;
        let over = rect.is_some_and(|r| areas.iter().any(|a| a.overlaps(&r)));
        if over {
            count += 1;
            if let Some(r) = rect.filter(|r| !areas.iter().any(|a| a.contains_rect(r))) {
                beyond.push([(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)]);
            }
            if let Obj::Ref(r) = item {
                gone.insert(r.num);
                if d.name(b"Subtype") == Some(b"Widget") {
                    removed_widgets.insert(r.num);
                }
            }
        } else {
            keep.push((item, d));
        }
    }
    if count == 0 {
        return Ok(0);
    }
    let mut kept = Vec::new();
    for (item, d) in keep {
        let orphan = d
            .get(b"Parent")
            .and_then(Obj::as_ref)
            .is_some_and(|p| gone.contains(&p.num))
            && d.name(b"Subtype") == Some(b"Popup");
        if orphan {
            count += 1;
            if let Obj::Ref(r) = item {
                erased.objects.insert(r.num);
            }
        } else {
            kept.push(item);
        }
    }
    page.set(b"Annots", Obj::Array(kept));
    erased.objects.extend(gone.iter().copied());
    Ok(count)
}

/// Removes form fields whose widgets were taken out, so a field's value does
/// not outlive the widget that showed it.
fn prune_form_fields(pdf: &mut Pdf<'_>, widgets: &BTreeSet<u32>, gone: &mut Gone) -> Result<()> {
    let (cat_ref, mut cat) = pdf.catalog()?;
    let Some(af_obj) = cat.get(b"AcroForm").cloned() else {
        return Ok(());
    };
    let Some(mut af) = pdf.resolve_dict(&af_obj)? else {
        return Ok(());
    };
    let Some(fields) = af.get(b"Fields").cloned() else {
        return Ok(());
    };
    let Obj::Array(items) = pdf.resolve(&fields)? else {
        return Ok(());
    };
    let mut visited = BTreeSet::new();
    let kept = prune_kids(pdf, items, widgets, &mut visited, 0, gone)?;
    af.set(b"Fields", Obj::Array(kept));
    match af_obj {
        Obj::Ref(r) => pdf.set(r.num, Stored::Obj(Obj::Dict(af))),
        _ => {
            cat.set(b"AcroForm", Obj::Dict(af));
            pdf.set(cat_ref.num, Stored::Obj(Obj::Dict(cat)));
        }
    }
    Ok(())
}

/// The kids that survive; a field left with no kids goes too.
fn prune_kids(
    pdf: &mut Pdf<'_>,
    items: Vec<Obj>,
    widgets: &BTreeSet<u32>,
    visited: &mut BTreeSet<u32>,
    depth: usize,
    gone: &mut Gone,
) -> Result<Vec<Obj>> {
    let mut out = Vec::new();
    for item in items {
        let Obj::Ref(r) = item else {
            out.push(item);
            continue;
        };
        if widgets.contains(&r.num) {
            continue;
        }
        if depth > 32 || !visited.insert(r.num) {
            out.push(item);
            continue;
        }
        let Some(mut d) = pdf.resolve_dict(&item)? else {
            continue;
        };
        if let Some(kids) = d.get(b"Kids").cloned() {
            if let Obj::Array(k) = pdf.resolve(&kids)? {
                let before = k.len();
                let left = prune_kids(pdf, k, widgets, visited, depth + 1, gone)?;
                if left.is_empty() && before > 0 {
                    // A field whose every widget went: its value goes too.
                    gone.objects.insert(r.num);
                    continue;
                }
                if left.len() != before {
                    d.set(b"Kids", Obj::Array(left));
                    pdf.set(r.num, Stored::Obj(Obj::Dict(d)));
                }
            }
        }
        out.push(item);
    }
    Ok(out)
}

/// In the structure tree, drops `/ActualText`, `/Alt` and `/E` from elements
/// whose marked content on this page lost something.
fn strip_structure(pdf: &mut Pdf<'_>, page: Ref, mcids: &[i64]) -> Result<()> {
    let (_, cat) = pdf.catalog()?;
    let Some(root) = cat.get(b"StructTreeRoot").cloned() else {
        return Ok(());
    };
    let Some(root) = pdf.resolve_dict(&root)? else {
        return Ok(());
    };
    let Some(k) = root.get(b"K").cloned() else {
        return Ok(());
    };
    let mcids: BTreeSet<i64> = mcids.iter().copied().collect();
    let mut todo: Vec<(Obj, Option<u32>, usize)> = vec![(k, None, 0)];
    let mut visited = BTreeSet::new();
    let mut steps = 0usize;
    while let Some((node, page_of_parent, depth)) = todo.pop() {
        steps += 1;
        if steps > 2_000_000 || depth > 256 {
            break;
        }
        match &node {
            Obj::Array(items) => {
                for i in items {
                    todo.push((i.clone(), page_of_parent, depth + 1));
                }
            }
            Obj::Ref(r) => {
                if !visited.insert(r.num) {
                    continue;
                }
                let Some(d) = pdf.resolve_dict(&node)? else {
                    continue;
                };
                let pg = d
                    .get(b"Pg")
                    .and_then(Obj::as_ref)
                    .map(|p| p.num)
                    .or(page_of_parent);
                let on_page = pg == Some(page.num);
                let hits = match d.get(b"K") {
                    Some(Obj::Int(m)) => on_page && mcids.contains(m),
                    Some(Obj::Array(a)) => a.iter().any(|x| match x {
                        Obj::Int(m) => on_page && mcids.contains(m),
                        Obj::Dict(mcr) => {
                            let mpg = mcr.get(b"Pg").and_then(Obj::as_ref).map(|p| p.num).or(pg);
                            mpg == Some(page.num)
                                && mcr
                                    .get(b"MCID")
                                    .and_then(Obj::as_i64)
                                    .is_some_and(|m| mcids.contains(&m))
                        }
                        _ => false,
                    }),
                    Some(Obj::Dict(mcr)) => {
                        let mpg = mcr.get(b"Pg").and_then(Obj::as_ref).map(|p| p.num).or(pg);
                        mpg == Some(page.num)
                            && mcr
                                .get(b"MCID")
                                .and_then(Obj::as_i64)
                                .is_some_and(|m| mcids.contains(&m))
                    }
                    _ => false,
                };
                if hits
                    && [&b"ActualText"[..], b"Alt", b"E"]
                        .iter()
                        .any(|k| d.contains(k))
                {
                    let mut clean = d.clone();
                    for k in [&b"ActualText"[..], b"Alt", b"E"] {
                        clean.remove(k);
                    }
                    pdf.set(r.num, Stored::Obj(Obj::Dict(clean)));
                }
                if let Some(k) = d.get(b"K") {
                    todo.push((k.clone(), pg, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Every XObject drawn anywhere: page contents, the forms they draw, tiling
/// patterns, soft masks, Type 3 glyphs, and annotation appearances.
fn drawn_everywhere(pdf: &Pdf<'_>) -> Result<BTreeSet<u32>> {
    let mut walk = Drawn {
        drawn: BTreeSet::new(),
        streams_seen: BTreeSet::new(),
    };
    for p in pdf.pages()? {
        let Some(page) = pdf.resolve_dict(&Obj::Ref(p))? else {
            continue;
        };
        let resources = match pdf.inherited(&page, b"Resources")? {
            Some(r) => pdf.resolve_dict(&r)?.unwrap_or_default(),
            None => Dict::new(),
        };
        let content = contents_of(pdf, &page)?;
        walk.content(pdf, &content, &resources, 0)?;
        if let Some(annots) = page.get(b"Annots") {
            if let Obj::Array(items) = pdf.resolve(annots)? {
                for a in items {
                    let Some(ad) = pdf.resolve_dict(&a)? else {
                        continue;
                    };
                    if let Some(ap) = ad.get(b"AP") {
                        if let Some(ap) = pdf.resolve_dict(ap)? {
                            for (_, v) in &ap.0 {
                                walk.appearance(pdf, v, &resources)?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(walk.drawn)
}

struct Drawn {
    drawn: BTreeSet<u32>,
    streams_seen: BTreeSet<u32>,
}

impl Drawn {
    /// An appearance entry: a stream, or a dictionary of state streams.
    fn appearance(&mut self, pdf: &Pdf<'_>, v: &Obj, parent: &Dict) -> Result<()> {
        if pdf.stream_of(v)?.is_some() {
            return self.stream(pdf, v, parent, 0);
        }
        if let Some(states) = pdf.resolve_dict(v)? {
            for (_, s) in &states.0 {
                self.stream(pdf, s, parent, 0)?;
            }
        }
        Ok(())
    }

    /// A content-bearing stream (form, pattern, glyph, mask) with its own
    /// resources or its parent's.
    fn stream(&mut self, pdf: &Pdf<'_>, r: &Obj, parent: &Dict, depth: usize) -> Result<()> {
        if depth > 12 {
            return Ok(());
        }
        if let Obj::Ref(r) = r {
            if !self.streams_seen.insert(r.num) {
                return Ok(());
            }
        }
        let Some(s) = pdf.stream_of(r)? else {
            return Ok(());
        };
        let resources = match s.dict.get(b"Resources") {
            Some(res) => pdf.resolve_dict(res)?.unwrap_or_default(),
            None => parent.clone(),
        };
        let Ok(content) = decode_plain(Some(pdf), &s.dict, &s.data) else {
            return Ok(());
        };
        self.content(pdf, &content, &resources, depth + 1)
    }

    fn content(
        &mut self,
        pdf: &Pdf<'_>,
        content: &[u8],
        resources: &Dict,
        depth: usize,
    ) -> Result<()> {
        let comps = |cs: &Obj| image::components(Some(pdf), cs, Some(resources));
        let ops = content::parse(content, &comps)?;
        let sub = |key: &[u8]| -> Result<Dict> {
            Ok(match resources.get(key) {
                Some(d) => pdf.resolve_dict(d)?.unwrap_or_default(),
                None => Dict::new(),
            })
        };
        let xobjects = sub(b"XObject")?;
        for op in &ops {
            if !op.is(b"Do") {
                continue;
            }
            let Some(name) = op.operands.first().and_then(Obj::as_name) else {
                continue;
            };
            let Some(Obj::Ref(r)) = xobjects.get(name) else {
                continue;
            };
            self.drawn.insert(r.num);
            let is_form = pdf
                .resolve_dict(&Obj::Ref(*r))?
                .is_some_and(|d| d.name(b"Subtype") == Some(b"Form"));
            if is_form {
                self.stream(pdf, &Obj::Ref(*r), resources, depth)?;
            }
        }
        // Whatever else in the resources can paint: tiling patterns, soft
        // mask groups, Type 3 glyph procedures. Walked whether or not this
        // stream uses them — erring towards "drawn" only keeps an object.
        for (_, p) in &sub(b"Pattern")?.0 {
            self.stream(pdf, p, resources, depth)?;
        }
        for (_, g) in &sub(b"ExtGState")?.0 {
            let Some(g) = pdf.resolve_dict(g)? else {
                continue;
            };
            if let Some(sm) = g.get(b"SMask") {
                if let Some(sm) = pdf.resolve_dict(sm)? {
                    if let Some(group) = sm.get(b"G") {
                        self.stream(pdf, group, resources, depth)?;
                    }
                }
            }
        }
        for (_, f) in &sub(b"Font")?.0 {
            let Some(fd) = pdf.resolve_dict(f)? else {
                continue;
            };
            if fd.name(b"Subtype") != Some(b"Type3") {
                continue;
            }
            let own = match fd.get(b"Resources") {
                Some(r) => pdf.resolve_dict(r)?.unwrap_or_default(),
                None => resources.clone(),
            };
            if let Some(procs) = fd.get(b"CharProcs") {
                if let Some(procs) = pdf.resolve_dict(procs)? {
                    for (_, cp) in &procs.0 {
                        self.stream(pdf, cp, &own, depth)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Objects referred to from anywhere other than a name in an `/XObject`
/// resource dictionary: an image used as another's `/SMask`, a form used as
/// an appearance. Those uses are not "drawing by name", and an object with
/// one is never erased.
fn references_outside_xobject_maps(pdf: &Pdf<'_>) -> Result<BTreeSet<u32>> {
    fn walk(obj: &Obj, maps: &mut BTreeSet<u32>, out: &mut BTreeSet<u32>) {
        match obj {
            Obj::Ref(r) => {
                out.insert(r.num);
            }
            Obj::Array(items) => items.iter().for_each(|i| walk(i, maps, out)),
            Obj::Dict(d) => {
                for (k, v) in &d.0 {
                    if k == b"XObject" {
                        match v {
                            // A map: its values are names for drawing.
                            Obj::Dict(_) => continue,
                            // An indirect map: remember it, skip it below.
                            Obj::Ref(r) => {
                                maps.insert(r.num);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    walk(v, maps, out);
                }
            }
            _ => {}
        }
    }
    let mut maps = BTreeSet::new();
    let mut per_object: Vec<(u32, BTreeSet<u32>)> = Vec::new();
    for num in pdf.reachable()? {
        let mut refs = BTreeSet::new();
        match pdf.get(num)? {
            Stored::Obj(o) => walk(&o, &mut maps, &mut refs),
            Stored::Stream(s) => walk(&Obj::Dict(s.dict), &mut maps, &mut refs),
        }
        per_object.push((num, refs));
    }
    let mut out = BTreeSet::new();
    for (num, refs) in per_object {
        if !maps.contains(&num) {
            out.extend(refs);
        }
    }
    Ok(out)
}
