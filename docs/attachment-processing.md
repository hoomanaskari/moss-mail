# Attachment text and OCR limits

Moss keeps the original attachment available even when it cannot extract text from it. These limits apply to indexing and text recognition, so an attachment that exceeds them may not appear in text searches.

- Extraction accepts at most 25 MiB of source data per attachment.
- PDF text extraction reads up to 100 pages. OCR reads up to 20 scanned pages.
- Image OCR accepts dimensions up to 16,384 pixels on either side and at most 25 million pixels in total. It reads only the first frame or page of an image file.
- Image samples must be 1 to 16 bits deep. A conservative estimate of five channels per pixel must fit within 128 MiB; this keeps smaller 16-bit images supported while rejecting large high-depth images.
- Before OCR, accepted images are resized to at most 2,000 pixels on their longest side, with their orientation applied. PDF pages use the same OCR size limit.
- The extracted text is limited to 100,000 characters per attachment.

Image dimensions are checked before pixel decoding. These limits bound the work Moss requests from the native parsers; they are not a promise of an exact memory or processing-time ceiling inside macOS frameworks.
