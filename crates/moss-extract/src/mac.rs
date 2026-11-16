//! The parts only macOS can do, on this Mac: the text layer of a PDF (PDFKit), and the words
//! in a picture or a scanned page (Vision's text recognition). Nothing leaves the Mac.

use objc2::rc::{autoreleasepool, Retained};
use objc2::AnyThread;
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSSize};
use objc2_image_io::{
    kCGImagePropertyDepth, kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize, CGImageSource,
};
use objc2_pdf_kit::{PDFDisplayBox, PDFDocument};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
};

use crate::{limited_pages, Native};

/// Pages of a scanned PDF that are read: a long scan is mostly the same form again.
pub const OCR_PAGES: usize = 20;
/// Sparse text PDFs cannot make the native parser traverse an unbounded page tree.
pub const TEXT_PAGES: usize = 100;
/// The longest side a scanned page is drawn at for reading: sharp enough for small print.
const PAGE_PIXELS: f64 = 2_000.0;
const MAX_IMAGE_PIXELS: i64 = 25_000_000;
const MAX_IMAGE_SIDE: i64 = 16_384;
const MAX_DECODED_SAMPLE_BYTES: i64 = 128 * 1024 * 1024;

/// PDFKit and Vision.
#[derive(Debug, Default, Clone, Copy)]
pub struct Mac;

fn document(bytes: &[u8]) -> Option<Retained<PDFDocument>> {
    let data = NSData::with_bytes(bytes);
    unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
}

fn valid_dimensions(width: i64, height: i64, depth: i64) -> bool {
    // Side and depth bounds make multiplication safe. Five channels conservatively
    // covers CMYK plus alpha, while retaining smaller 16-bit images.
    width > 0
        && height > 0
        && width <= MAX_IMAGE_SIDE
        && height <= MAX_IMAGE_SIDE
        && width * height <= MAX_IMAGE_PIXELS
        && (1..=16).contains(&depth)
        && width * height * ((depth + 7) / 8) * 5 <= MAX_DECODED_SAMPLE_BYTES
}

/// Read image metadata with caching disabled, before allowing any pixel decoding.
fn image_source(bytes: &[u8]) -> Option<CFRetained<CGImageSource>> {
    if bytes.len() > crate::MAX_INPUT_BYTES {
        return None;
    }
    unsafe {
        let options =
            CFDictionary::from_slices(&[kCGImageSourceShouldCache], &[CFBoolean::new(false)]);
        let source =
            CGImageSource::with_data(&CFData::from_bytes(bytes), Some(options.as_opaque()))?;
        let properties = source.properties_at_index(0, Some(options.as_opaque()))?;
        // ImageIO returns a dictionary of CFString keys and Core Foundation values.
        let properties = properties.cast_unchecked::<CFString, CFType>();
        let width = properties
            .get(kCGImagePropertyPixelWidth)?
            .downcast_ref::<CFNumber>()?
            .as_i64()?;
        let height = properties
            .get(kCGImagePropertyPixelHeight)?
            .downcast_ref::<CFNumber>()?
            .as_i64()?;
        let depth = properties
            .get(kCGImagePropertyDepth)?
            .downcast_ref::<CFNumber>()?
            .as_i64()?;
        valid_dimensions(width, height, depth).then_some(source)
    }
}

fn image_for_ocr(bytes: &[u8]) -> Option<CFRetained<CGImage>> {
    let source = image_source(bytes)?;
    let size = CFNumber::new_i64(PAGE_PIXELS as i64);
    unsafe {
        let options = CFDictionary::<CFString, CFType>::from_slices(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceThumbnailMaxPixelSize,
                kCGImageSourceShouldCache,
            ],
            &[
                CFBoolean::new(true).as_ref(),
                CFBoolean::new(true).as_ref(),
                size.as_ref(),
                CFBoolean::new(false).as_ref(),
            ],
        );
        source.thumbnail_at_index(0, Some(options.as_opaque()))
    }
}

/// Vision only receives the bounded, orientation-corrected first image.
fn recognize(bytes: &[u8]) -> Option<String> {
    let image = image_for_ocr(bytes)?;
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            &image,
            &NSDictionary::new(),
        )
    };
    let request = unsafe { VNRecognizeTextRequest::init(VNRecognizeTextRequest::alloc()) };
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    request.setAutomaticallyDetectsLanguage(true);
    let requests: Retained<NSArray<VNRequest>> =
        NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(
            request.clone(),
        ))]);
    handler.performRequests_error(&requests).ok()?;
    let lines: Vec<String> = request
        .results()?
        .iter()
        .filter_map(|o| {
            o.topCandidates(1)
                .firstObject()
                .map(|t| t.string().to_string())
        })
        .collect();
    Some(lines.join("\n"))
}

impl Native for Mac {
    fn pdf_text(&self, bytes: &[u8]) -> Option<String> {
        autoreleasepool(|_| {
            let pdf = document(bytes)?;
            Some(limited_pages(
                unsafe { pdf.pageCount() }.min(TEXT_PAGES),
                |index| {
                    autoreleasepool(|_| {
                        unsafe { pdf.pageAtIndex(index)?.string() }.map(|text| text.to_string())
                    })
                },
            ))
        })
    }

    fn pdf_ocr(&self, bytes: &[u8]) -> Option<String> {
        autoreleasepool(|_| {
            let pdf = document(bytes)?;
            let pages = unsafe { pdf.pageCount() }.min(OCR_PAGES);
            Some(limited_pages(pages, |n| {
                autoreleasepool(|_| {
                    let page = unsafe { pdf.pageAtIndex(n) }?;
                    let bounds = unsafe { page.boundsForBox(PDFDisplayBox::MediaBox) };
                    let scale = PAGE_PIXELS / bounds.size.width.max(bounds.size.height).max(1.0);
                    let size = NSSize::new(bounds.size.width * scale, bounds.size.height * scale);
                    let image =
                        unsafe { page.thumbnailOfSize_forBox(size, PDFDisplayBox::MediaBox) };
                    let image_data = image.TIFFRepresentation()?;
                    recognize(&image_data.to_vec())
                })
            }))
        })
    }

    fn image_ocr(&self, bytes: &[u8]) -> Option<String> {
        autoreleasepool(|_| recognize(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pdf;

    /// A page of `pdf` drawn as a TIFF, like a photo of it.
    fn picture(pdf: &[u8]) -> Vec<u8> {
        picture_at_size(pdf, NSSize::new(1_200.0, 1_600.0))
    }

    fn picture_at_size(pdf: &[u8], size: NSSize) -> Vec<u8> {
        autoreleasepool(|_| unsafe {
            let document = document(pdf).unwrap();
            let page = document.pageAtIndex(0).unwrap();
            let image = page.thumbnailOfSize_forBox(size, PDFDisplayBox::MediaBox);
            image.TIFFRepresentation().unwrap().to_vec()
        })
    }

    fn compressed_png(width: u32, height: u32) -> Vec<u8> {
        png_with_depth(width, height, 8, 2)
    }

    fn png_with_depth(width: u32, height: u32, depth: u8, color: u8) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(data);
            let mut crc = crc32fast::Hasher::new();
            crc.update(name);
            crc.update(data);
            out.extend_from_slice(&crc.finalize().to_be_bytes());
        }
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[depth, color, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &header);
        // Stream compressed pixels so even the over-budget fixture is small in memory.
        use std::io::Write;
        let mut data = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        let channels = if color == 6 { 4 } else { 3 };
        let row = vec![0; width as usize * channels * usize::from(depth / 8) + 1];
        for _ in 0..height {
            data.write_all(&row).unwrap();
        }
        chunk(&mut bytes, b"IDAT", &data.finish().unwrap());
        chunk(&mut bytes, b"IEND", &[]);
        bytes
    }

    #[test]
    fn oversized_image_headers_are_rejected_before_decoding() {
        for (width, height) in [(5_001, 5_000), (20_000, 1)] {
            let bytes = compressed_png(width, height);
            assert!(bytes.len() < 100_000);
            assert!(
                image_source(&bytes).is_none(),
                "{width} by {height} must not reach decoding"
            );
        }
        assert!(
            image_source(&compressed_png(5_000, 5_000)).is_some(),
            "the boundary header must contain readable dimensions"
        );
        assert!(image_source(&picture(&pdf(&["Small document"]))).is_some());
    }

    #[test]
    fn high_depth_images_must_fit_the_decoded_sample_budget() {
        let oversized = png_with_depth(5_000, 5_000, 16, 6);
        assert!(oversized.len() < crate::MAX_INPUT_BYTES);
        assert!(
            image_source(&oversized).is_none(),
            "a 16-bit RGBA bitmap must not bypass the decode budget"
        );
        assert!(
            image_source(&png_with_depth(1_000, 1_000, 16, 6)).is_some(),
            "smaller high-depth images remain supported"
        );
    }

    #[test]
    fn image_dimension_limits_cover_boundaries_and_overflow() {
        for (width, height, allowed) in [
            (1, 1, true),
            (5_000, 5_000, true),
            (5_001, 5_000, false),
            (0, 1, false),
            (1, 0, false),
            (-1, 1, false),
            (1, -1, false),
            (16_384, 1, true),
            (16_385, 1, false),
            (1, 16_385, false),
            (i64::MAX, i64::MAX, false),
        ] {
            assert_eq!(
                valid_dimensions(width, height, 8),
                allowed,
                "{width} by {height}"
            );
        }
    }

    #[test]
    fn image_depth_limits_fail_closed_and_keep_smaller_high_depth_images() {
        for depth in [i64::MIN, -1, 0, 17, i64::MAX] {
            assert!(!valid_dimensions(1, 1, depth));
        }
        assert!(valid_dimensions(5_000, 2_600, 16));
        assert!(!valid_dimensions(5_000, 2_700, 16));
        assert!(valid_dimensions(1, 1, 1));
    }

    #[test]
    fn ocr_decodes_a_bounded_thumbnail_and_rejects_oversized_input() {
        let bytes = compressed_png(2_500, 1_000);
        let image = image_for_ocr(&bytes).unwrap();
        let width = CGImage::width(Some(&image));
        let height = CGImage::height(Some(&image));
        assert!(width > 0 && height > 0);
        assert!(width <= 2_000 && height <= 2_000, "{width} by {height}");
        let mut oversized = picture(&pdf(&["Small document"]));
        oversized.resize(crate::MAX_INPUT_BYTES + 1, 0);
        assert!(image_source(&oversized).is_none());
        assert!(image_for_ocr(b"not an image").is_none());
    }

    #[test]
    fn pdf_text_does_not_read_past_its_page_budget() {
        let bytes = autoreleasepool(|_| unsafe {
            let doc = document(&pdf(&["Allowed page"])).unwrap();
            for index in 1..=100 {
                let text = if index == 100 {
                    "Outside page budget"
                } else {
                    "Allowed page"
                };
                let next = document(&pdf(&[text])).unwrap();
                doc.insertPage_atIndex(&next.pageAtIndex(0).unwrap(), index);
            }
            doc.dataRepresentation().unwrap().to_vec()
        });
        let text = Mac.pdf_text(&bytes).unwrap();
        assert!(text.contains("Allowed page"));
        assert!(!text.contains("Outside page budget"));
    }

    #[test]
    fn reads_the_text_layer_of_a_pdf() {
        let text = Mac
            .pdf_text(&pdf(&[
                "Resident Parking Permit",
                "Date: December 12, 2026",
            ]))
            .unwrap();
        assert!(text.contains("Resident Parking Permit"), "{text}");
        assert!(text.contains("December 12, 2026"), "{text}");
        assert_eq!(Mac.pdf_text(b"not a pdf"), None);
    }

    #[test]
    fn reads_scanned_pages_and_pictures_by_how_they_look() {
        let invitation = pdf(&["Resident Parking Permit", "Date: December 12, 2026"]);
        let scanned = Mac.pdf_ocr(&invitation).unwrap();
        assert!(scanned.contains("Parking Permit"), "{scanned}");
        let photo = Mac.image_ocr(&picture(&invitation)).unwrap();
        assert!(photo.contains("December 12, 2026"), "{photo}");
        assert_eq!(Mac.pdf_ocr(b"not a pdf"), None);
        assert_eq!(Mac.image_ocr(b"not a picture"), None);
    }
}
