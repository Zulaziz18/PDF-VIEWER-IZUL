//! Files embedded in the document (SPEC 11.1's "Lampiran" sidebar), found
//! missing in the Phase 8 audit.
//!
//! Only reading: the list with each file's name and size, and a file's bytes
//! to save somewhere. Adding and deleting attachments is not offered — PDFium's
//! own header says deleting "does not remove the attachment data from the PDF
//! file", which is not what a user who deletes something means.

use std::ffi::c_void;
use std::os::raw::c_ulong;

use crate::engine::Document;
use crate::error::{PdfError, Result};

/// The largest attachment handed over: generous, and still a cap — the bytes
/// cross to the UI process whole.
pub const MAX_ATTACHMENT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInfo {
    pub name: String,
    /// Bytes of the embedded file, or 0 when PDFium cannot read it.
    pub size: u64,
}

impl Document {
    /// Every embedded file, in the document's order.
    pub fn attachments(&self) -> Result<Vec<AttachmentInfo>> {
        let b = self.engine().bindings();
        // SAFETY: the handle is a live document.
        let n = unsafe { b.FPDFDoc_GetAttachmentCount(self.handle()) }.max(0);
        let mut out = Vec::new();
        for i in 0..n {
            // SAFETY: `i` is below the count just read.
            let att = unsafe { b.FPDFDoc_GetAttachment(self.handle(), i) };
            if att.is_null() {
                continue;
            }
            // Name: the two-call protocol, UTF-16LE with a terminator.
            // SAFETY: a null buffer with length 0 asks for the size only.
            let bytes = unsafe { b.FPDFAttachment_GetName(att, std::ptr::null_mut(), 0) } as usize;
            let name = if (2..=(1 << 16)).contains(&bytes) {
                let mut buf = vec![0u16; bytes.div_ceil(2)];
                // SAFETY: `buf` holds `bytes` bytes, as the query asked.
                let written =
                    unsafe { b.FPDFAttachment_GetName(att, buf.as_mut_ptr(), bytes as c_ulong) }
                        as usize;
                let units = (written / 2).saturating_sub(1).min(buf.len());
                String::from_utf16_lossy(buf.get(..units).unwrap_or(&[]))
            } else {
                String::new()
            };
            let mut size: c_ulong = 0;
            // SAFETY: a null buffer asks for the length into `size`.
            let ok = unsafe { b.FPDFAttachment_GetFile(att, std::ptr::null_mut(), 0, &mut size) };
            out.push(AttachmentInfo {
                name,
                size: if ok != 0 { size as u64 } else { 0 },
            });
        }
        Ok(out)
    }

    /// The bytes of embedded file `index`.
    pub fn attachment_bytes(&self, index: u32) -> Result<Vec<u8>> {
        let b = self.engine().bindings();
        let unreadable = || PdfError::Attachment {
            index,
            detail: "tidak terbaca".into(),
        };
        // SAFETY: the handle is a live document; PDFium checks the index.
        let att = unsafe { b.FPDFDoc_GetAttachment(self.handle(), index as i32) };
        if att.is_null() {
            return Err(unreadable());
        }
        let mut size: c_ulong = 0;
        // SAFETY: a null buffer asks for the length into `size`.
        if unsafe { b.FPDFAttachment_GetFile(att, std::ptr::null_mut(), 0, &mut size) } == 0 {
            return Err(unreadable());
        }
        if size as u64 > MAX_ATTACHMENT {
            return Err(PdfError::Attachment {
                index,
                detail: format!("terlalu besar ({} MB)", size as u64 / (1024 * 1024)),
            });
        }
        let mut buf = vec![0u8; size as usize];
        let mut written: c_ulong = 0;
        // SAFETY: `buf` holds `size` bytes, the length PDFium reported.
        let ok = unsafe {
            b.FPDFAttachment_GetFile(att, buf.as_mut_ptr() as *mut c_void, size, &mut written)
        };
        if ok == 0 {
            return Err(unreadable());
        }
        buf.truncate(written as usize);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use crate::engine_and_lock;

    /// A one-page PDF carrying `files` in its /EmbeddedFiles name tree, with a
    /// correct cross-reference table (so nothing here leans on PDFium's repair).
    pub(crate) fn pdf_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut objs: Vec<Vec<u8>> = Vec::new();
        let n_fixed = 3; // catalog, pages, page
        let mut names = String::new();
        for (i, (name, _)) in files.iter().enumerate() {
            let spec = n_fixed + 1 + i * 2; // filespec object number
            names.push_str(&format!("({name}) {spec} 0 R "));
        }
        objs.push(
            format!("<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [{names}] >> >> >>")
                .into_bytes(),
        );
        objs.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
        objs.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_vec());
        for (i, (name, data)) in files.iter().enumerate() {
            let stream = n_fixed + 2 + i * 2;
            objs.push(
                format!("<< /Type /Filespec /F ({name}) /UF ({name}) /EF << /F {stream} 0 R >> >>")
                    .into_bytes(),
            );
            let mut s =
                format!("<< /Type /EmbeddedFile /Length {} >>\nstream\n", data.len()).into_bytes();
            s.extend_from_slice(data);
            s.extend_from_slice(b"\nendstream");
            objs.push(s);
        }
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn lists_embedded_files_and_hands_over_their_bytes() {
        let (engine, _pdfium) = engine_and_lock!();
        let data: &[u8] = b"Nomor;Nama\n1;Ayu\n";
        let doc = engine
            .open_bytes(
                pdf_with(&[("data.csv", data), ("catatan.txt", b"halo")]),
                None,
                None,
            )
            .expect("buka");
        let list = doc.attachments().expect("daftar");
        let got: Vec<(&str, u64)> = list.iter().map(|a| (a.name.as_str(), a.size)).collect();
        assert_eq!(
            got,
            vec![("data.csv", data.len() as u64), ("catatan.txt", 4)]
        );
        assert_eq!(doc.attachment_bytes(0).expect("isi"), data);
        assert_eq!(doc.attachment_bytes(1).expect("isi"), b"halo");
        assert!(
            doc.attachment_bytes(7).is_err(),
            "indeks di luar daftar ditolak"
        );
    }

    #[test]
    fn a_document_without_attachments_lists_none() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open_bytes(pdf_with(&[]), None, None).expect("buka");
        assert!(doc.attachments().expect("daftar").is_empty());
    }
}
