//! From PDFium's saved bytes plus our objects to the finished file.

use std::collections::HashMap;

use izul_model::annot::AnnotObject;
use izul_model::ap::{appearance, GState};
use izul_model::display::{BlendMode, DisplayList, FontRef, ImageRef};

use crate::annot_dict::annotation_dict;
use crate::bounds::painted_bounds;
use crate::images::{deflate, image_objects, stream, SMASK_SLOT};
use crate::incremental::{find_placeholders, read_tail, Placement, SyntaxError, Update};
use crate::syntax::{name, num, rect};

/// One annotation to write: the object, and the display list built from it
/// with the font context the editor used — the same list the canvas drew.
#[derive(Debug)]
pub struct AnnotWrite<'a> {
    pub obj: &'a AnnotObject,
    pub list: &'a DisplayList,
}

/// What the lists refer to by handle.
pub trait Assets {
    /// The standard-14 `/BaseFont` for a font handle.
    fn base_font(&self, font: FontRef) -> Option<&'static str>;
    /// The original bytes of an inserted image.
    fn image(&self, image: ImageRef) -> Option<Vec<u8>>;
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
    #[error("anotasi {0} hilang dari hasil PDFium")]
    MissingPlaceholder(u64),
    #[error("font untuk anotasi {0} tidak diketahui")]
    UnknownFont(u64),
    #[error("gambar untuk anotasi {0} tidak ada: {1}")]
    Image(u64, String),
}

fn blend_name(b: BlendMode) -> &'static str {
    match b {
        BlendMode::Normal => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
    }
}

fn gstate(state: &GState) -> String {
    format!(
        "<</Type/ExtGState/ca {}/CA {}/BM/{}>>",
        num(state.fill_alpha),
        num(state.stroke_alpha),
        blend_name(state.blend)
    )
}

/// An object that held inline placeholders: its generation, the byte range of
/// its body, and each placeholder's `<<…>>` span with the number of the object
/// that replaces it.
type Container = (u16, (usize, usize), Vec<((usize, usize), u32)>);

/// Appends the annotations to `pdf`, which must be `FPDF_SaveAsCopy` output
/// carrying one placeholder per annotation (`/NM (izul-<id>)`).
///
/// Fonts and images are written once however many annotations use them.
pub fn patch(
    pdf: Vec<u8>,
    annots: &[AnnotWrite<'_>],
    assets: &dyn Assets,
) -> Result<Vec<u8>, SaveError> {
    if annots.is_empty() {
        return Ok(pdf);
    }
    let (trailer, xref) = read_tail(&pdf)?;
    let ids: Vec<u64> = annots.iter().map(|a| a.obj.id.0).collect();
    let placeholders = find_placeholders(&pdf, &xref, &ids);

    let mut up = Update::new(&trailer);
    let mut fonts: HashMap<&'static str, u32> = HashMap::new();
    let mut images: HashMap<u32, u32> = HashMap::new();
    // Containers of inline placeholders: object -> (generation, body range,
    // replacements of `<<…>>` spans by references).
    let mut containers: HashMap<u32, Container> = HashMap::new();

    for a in annots {
        let id = a.obj.id.0;
        let placement = *placeholders
            .get(&id)
            .ok_or(SaveError::MissingPlaceholder(id))?;
        let (annot_num, annot_gen) = match placement {
            Placement::Object { num, gen } => (num, gen),
            Placement::Inline {
                container,
                gen,
                span,
                body,
            } => {
                // The annotation becomes an object of its own, and the page
                // refers to it where the inline dictionary was.
                let n = up.reserve();
                containers
                    .entry(container)
                    .or_insert_with(|| (gen, body, Vec::new()))
                    .2
                    .push((span, n));
                (n, 0)
            }
        };
        let bbox = painted_bounds(a.list, 1.0).unwrap_or(a.obj.rect);
        let ap = appearance(a.list, bbox);

        let mut res = String::from("<<");
        if !ap.resources.fonts.is_empty() {
            res.push_str("/Font<<");
            for (res_name, font) in &ap.resources.fonts {
                let base = assets.base_font(*font).ok_or(SaveError::UnknownFont(id))?;
                let num_ = match fonts.get(base) {
                    Some(n) => *n,
                    None => {
                        let n = up.reserve();
                        up.put(
                            n,
                            0,
                            format!(
                                "<</Type/Font/Subtype/Type1/BaseFont{}/Encoding/WinAnsiEncoding>>",
                                name(base)
                            )
                            .into_bytes(),
                        );
                        fonts.insert(base, n);
                        n
                    }
                };
                res.push_str(&format!("{} {num_} 0 R", name(res_name)));
            }
            res.push_str(">>");
        }
        if !ap.resources.images.is_empty() {
            res.push_str("/XObject<<");
            for (res_name, image) in &ap.resources.images {
                let num_ = match images.get(&image.0) {
                    Some(n) => *n,
                    None => {
                        let bytes = assets
                            .image(*image)
                            .ok_or_else(|| SaveError::Image(id, "tidak terdaftar".into()))?;
                        let objs = image_objects(&bytes)
                            .map_err(|e| SaveError::Image(id, e.to_string()))?;
                        let n = up.reserve();
                        let mut body = objs.image;
                        if let Some(mask) = objs.smask {
                            let m = up.reserve();
                            up.put(m, 0, mask);
                            let text = format!("{m} 0 R");
                            body = replace_once(&body, SMASK_SLOT.as_bytes(), text.as_bytes());
                        }
                        up.put(n, 0, body);
                        images.insert(image.0, n);
                        n
                    }
                };
                res.push_str(&format!("{} {num_} 0 R", name(res_name)));
            }
            res.push_str(">>");
        }
        if !ap.resources.states.is_empty() {
            res.push_str("/ExtGState<<");
            for (res_name, state) in &ap.resources.states {
                res.push_str(&format!("{}{}", name(res_name), gstate(state)));
            }
            res.push_str(">>");
        }
        res.push_str(">>");

        let ap_num = up.reserve();
        let form = format!(
            "/Type/XObject/Subtype/Form/BBox{}/Matrix[1 0 0 1 0 0]/Resources{res}/Filter/FlateDecode",
            rect(ap.bbox)
        );
        up.put(ap_num, 0, stream(&form, &deflate(ap.content.as_bytes())));

        let metadata = crate::metadata::encode(a.obj);
        up.put(
            annot_num,
            annot_gen,
            annotation_dict(a.obj, bbox, ap_num, &metadata).into_bytes(),
        );
    }
    for (num, (gen, (start, end), mut swaps)) in containers {
        swaps.sort_by_key(|(span, _)| std::cmp::Reverse(span.0));
        let mut body = pdf.get(start..end).unwrap_or_default().to_vec();
        for ((s, e), n) in swaps {
            let (s, e) = (s - start, e - start);
            if e > body.len() || s > e {
                return Err(SaveError::Syntax(SyntaxError::Xref(
                    "rentang placeholder".into(),
                )));
            }
            // Spaces on both sides: PDFium writes inline dictionaries back to
            // back (`>><<`), and `5 0 R7 0 R` would read as the token `R7`.
            body.splice(s..e, format!(" {n} 0 R ").into_bytes());
        }
        up.put(num, gen, body);
    }
    Ok(up.finish(pdf, &trailer))
}

fn replace_once(hay: &[u8], needle: &[u8], with: &[u8]) -> Vec<u8> {
    match hay.windows(needle.len()).position(|w| w == needle) {
        Some(at) => {
            let mut out = Vec::with_capacity(hay.len() + with.len());
            out.extend_from_slice(hay.get(..at).unwrap_or_default());
            out.extend_from_slice(with);
            out.extend_from_slice(hay.get(at + needle.len()..).unwrap_or_default());
            out
        }
        None => hay.to_vec(),
    }
}
