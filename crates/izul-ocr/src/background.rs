//! "Hapus Latar" for pictures (Phase 7): the background of an image made
//! transparent by a segmentation model, run on this machine.
//!
//! The model is U²-Net (small, `u2netp`, Apache-2.0), in ONNX, run by ONNX
//! Runtime — with DirectML on Windows, so a GPU does the work when there is
//! one, and the CPU otherwise. Which one actually ran is reported, never
//! assumed: DirectML is asked for with `error_on_failure`, and only when it
//! refuses does the session fall back to the CPU, saying why.
//!
//! Pre- and post-processing follow what the model was trained with (and what
//! rembg, where these weights are published, does): the picture scaled to
//! 320×320, divided by its brightest value, normalised with the ImageNet mean
//! and deviation; the first output min-max normalised into a mask, scaled back
//! to the picture's size, and used as its alpha.

use std::path::Path;

use image::imageops::FilterType;
use image::{GrayImage, ImageBuffer, Luma, RgbaImage};
use ort::session::Session;
use ort::value::Tensor;

use crate::OcrError;

const SIDE: u32 = 320;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// What the picture is, as the user says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    /// A photo of an object: the model's mask as it is.
    Photo,
    /// A signature or stamp on paper: the paper goes too.
    OnPaper,
}

/// Where the model ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Device {
    DirectMl,
    /// The CPU, and why not DirectML (empty where DirectML does not exist).
    Cpu {
        reason: String,
    },
}

pub struct BackgroundRemover {
    session: Session,
    device: Device,
}

impl std::fmt::Debug for BackgroundRemover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundRemover")
            .field("device", &self.device)
            .finish()
    }
}

fn engine(e: impl std::fmt::Display) -> OcrError {
    OcrError::Engine(e.to_string())
}

impl BackgroundRemover {
    /// Loads ONNX Runtime from `runtime` (the shared library) and the model.
    /// The runtime can be loaded once per process; a second call with another
    /// path keeps the first.
    pub fn load(runtime: &Path, model: &Path) -> Result<Self, OcrError> {
        Self::load_on(runtime, model, false)
    }

    /// [`load`](Self::load), with DirectML allowed on any adapter rather than
    /// a GPU only — including Windows' software rasteriser (WARP). Slower
    /// than the CPU there; it exists so that CI, which has no GPU, can run
    /// the model through DirectML at all.
    pub fn load_on(runtime: &Path, model: &Path, any_adapter: bool) -> Result<Self, OcrError> {
        if !runtime.is_file() {
            return Err(OcrError::Model {
                path: runtime.display().to_string(),
                detail: "ONNX Runtime tidak ada".into(),
            });
        }
        if !model.is_file() {
            return Err(OcrError::Model {
                path: model.display().to_string(),
                detail: "model tidak ada".into(),
            });
        }
        // Once per process: the library stays loaded, and the environment is
        // shared by every session after it.
        let _ = ort::init_from(runtime).map(|b| b.with_name("izul").commit());

        let (session, device) = if cfg!(windows) {
            let directml = Session::builder()
                .map_err(engine)?
                .with_execution_providers([{
                    let ep = ort::ep::DirectML::default();
                    let ep = if any_adapter {
                        ep.with_device_filter(ort::ep::directml::DeviceFilter::Any)
                    } else {
                        ep
                    };
                    ep.build().error_on_failure()
                }]);
            match directml {
                Ok(mut b) => match b.commit_from_file(model) {
                    Ok(s) => (s, Device::DirectMl),
                    Err(e) => (
                        Self::cpu(model)?,
                        Device::Cpu {
                            reason: e.to_string(),
                        },
                    ),
                },
                Err(e) => (
                    Self::cpu(model)?,
                    Device::Cpu {
                        reason: e.to_string(),
                    },
                ),
            }
        } else {
            (
                Self::cpu(model)?,
                Device::Cpu {
                    reason: String::new(),
                },
            )
        };
        Ok(BackgroundRemover { session, device })
    }

    fn cpu(model: &Path) -> Result<Session, OcrError> {
        Session::builder()
            .map_err(engine)?
            .commit_from_file(model)
            .map_err(engine)
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The mask of `picture`: 255 where the subject is, 0 where background.
    pub fn mask(&mut self, picture: &RgbaImage) -> Result<GrayImage, OcrError> {
        let (w, h) = picture.dimensions();
        if w == 0 || h == 0 {
            return Err(OcrError::Image("gambar kosong".into()));
        }
        let small = image::imageops::resize(picture, SIDE, SIDE, FilterType::Lanczos3);
        let brightest = small
            .pixels()
            .flat_map(|p| p.0.into_iter().take(3))
            .max()
            .unwrap_or(255)
            .max(1);
        let scale = 1.0 / f32::from(brightest);
        let side = SIDE as usize;
        // Channels first (NCHW), as the model takes them.
        let mut planar = Vec::with_capacity(3 * side * side);
        for (c, (mean, std)) in MEAN.iter().zip(STD).enumerate() {
            planar.extend(small.pixels().map(|p| {
                let v = p.0.get(c).copied().unwrap_or(0);
                (f32::from(v) * scale - mean) / std
            }));
        }
        let input = ndarray::Array4::from_shape_vec((1, 3, side, side), planar)
            .map_err(|e| OcrError::Engine(e.to_string()))?;
        let tensor = Tensor::from_array(input).map_err(engine)?;
        let outputs = self.session.run(ort::inputs![tensor]).map_err(engine)?;
        let first = outputs
            .values()
            .next()
            .ok_or_else(|| OcrError::Engine("model tidak mengeluarkan apa pun".into()))?;
        let (shape, data) = first.try_extract_tensor::<f32>().map_err(engine)?;
        if data.len() != side * side {
            return Err(OcrError::Engine(format!(
                "bentuk keluaran tak terduga: {shape:?}"
            )));
        }
        let (lo, hi) = data
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let span = (hi - lo).max(1e-6);
        let small_mask: GrayImage = ImageBuffer::from_fn(SIDE, SIDE, |x, y| {
            let v = data
                .get(y as usize * side + x as usize)
                .copied()
                .unwrap_or(0.0);
            Luma([(((v - lo) / span) * 255.0).round().clamp(0.0, 255.0) as u8])
        });
        Ok(image::imageops::resize(
            &small_mask,
            w,
            h,
            FilterType::Lanczos3,
        ))
    }

    /// `picture` with its background made transparent (the mask multiplied
    /// into whatever alpha it had), and whether the paper refinement ran.
    ///
    /// The user says which kind of picture it is. It cannot be told from the
    /// picture: a stamp on paper and a white cup on a white table both have a
    /// plain light edge, and the paper refinement that makes the first
    /// perfect (IoU 0.31 → 1.00) erases the second completely (1.00 → 0.00),
    /// measured on `tools/background-proof`.
    pub fn remove(
        &mut self,
        picture: &RgbaImage,
        kind: Kind,
    ) -> Result<(RgbaImage, bool), OcrError> {
        let model = self.mask(picture)?;
        let (mask, refined) = match kind {
            Kind::Photo => (model, false),
            Kind::OnPaper => match on_paper(picture, &model) {
                Some(m) => (m, true),
                None => (model, false),
            },
        };
        let mut out = picture.clone();
        for (p, m) in out.pixels_mut().zip(mask.pixels()) {
            p.0[3] = ((u16::from(p.0[3]) * u16::from(m.0[0]) + 127) / 255) as u8;
        }
        Ok((out, refined))
    }
}

/// Chromaticity (r, g share of r + g + b) — what shading on paper leaves
/// alone and ink does not.
fn chroma(p: [u8; 3]) -> (f32, f32) {
    let s = (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).max(1) as f32;
    (f32::from(p[0]) / s, f32::from(p[1]) / s)
}

fn lum(p: [u8; 3]) -> f32 {
    (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0
}

/// Paper under ink: when the picture's edge is plain and light, the paper
/// inside the model's mask goes too.
///
/// U²-Net finds *the object*, and to it a stamp is a disc: the paper inside
/// the ring stays (measured on `tools/background-proof`: IoU 0.31, a third of
/// the background kept). A signature or a stamp photographed on paper is the
/// picture people most often put into a PDF, so for those the mask is refined
/// by colour: a pixel with the paper's chromaticity and not much darker than
/// it is paper, whatever the light did to it. Only when the edge says
/// "paper": a chromaticity spread below 0.012 (paper measured 0.0004,
/// textured backgrounds 0.036–0.053) and light (mean above 150). Returns
/// `None` otherwise, and the model's mask stands.
pub fn on_paper(picture: &RgbaImage, model: &GrayImage) -> Option<GrayImage> {
    let (w, h) = picture.dimensions();
    let edge = 6u32.min(w / 4).min(h / 4).max(1);
    let border: Vec<[u8; 3]> = picture
        .enumerate_pixels()
        .filter(|(x, y, _)| *x < edge || *y < edge || *x >= w - edge || *y >= h - edge)
        .map(|(_, _, p)| [p.0[0], p.0[1], p.0[2]])
        .collect();
    if border.is_empty() {
        return None;
    }
    let n = border.len() as f32;
    let cs: Vec<(f32, f32)> = border.iter().map(|p| chroma(*p)).collect();
    let (mr, mg) = cs
        .iter()
        .fold((0.0, 0.0), |(a, b), c| (a + c.0 / n, b + c.1 / n));
    let spread = cs.iter().fold((0.0f32, 0.0f32), |(a, b), c| {
        (a + (c.0 - mr).powi(2) / n, b + (c.1 - mg).powi(2) / n)
    });
    let paper_lum = border.iter().map(|p| lum(*p)).sum::<f32>() / n;
    if spread.0.sqrt().max(spread.1.sqrt()) >= 0.012 || paper_lum <= 150.0 {
        return None;
    }
    let mut out = model.clone();
    for ((_, _, p), m) in picture.enumerate_pixels().zip(out.pixels_mut()) {
        let rgb = [p.0[0], p.0[1], p.0[2]];
        let (r, g) = chroma(rgb);
        let off_colour = ((r - mr).abs().max((g - mg).abs()) - 0.012) / 0.03;
        let darker = (0.72 - lum(rgb) / paper_lum) / 0.2;
        let ink = off_colour.max(darker).clamp(0.0, 1.0);
        m.0[0] = (f32::from(m.0[0]) * ink).round() as u8;
    }
    Some(out)
}
