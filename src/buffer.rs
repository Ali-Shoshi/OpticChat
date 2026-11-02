use anyhow::Result;
#[cfg(test)]
use image::DynamicImage;
use image::imageops::FilterType;
use std::{collections::VecDeque, sync::Arc, time::Instant};

#[derive(Clone)]
pub struct Frame {
    pub jpeg: Arc<Vec<u8>>,
    pub captured: Instant,
}
/// Decode/resize work happens before locking the shared buffer. The capture
/// helper already compressed this image, so preserve its bytes and quality.
#[derive(Clone)]
pub struct PreparedFrame {
    frame: Frame,
    signature: Arc<[u8]>,
}
impl PreparedFrame {
    pub fn from_jpeg(jpeg: Vec<u8>, captured: Instant) -> Result<Self> {
        anyhow::ensure!(
            jpeg.len() <= 1_000_000,
            "Compressed screenshot exceeds 1 MB; lower image width or quality"
        );
        let image = image::load_from_memory(&jpeg)?;
        let signature = image
            .resize_exact(64, 36, FilterType::Triangle)
            .to_rgb8()
            .into_raw()
            .into();
        Ok(Self {
            frame: Frame {
                jpeg: Arc::new(jpeg),
                captured,
            },
            signature,
        })
    }
    pub fn matches(&self, jpeg: &[u8]) -> bool {
        self.frame.jpeg.as_slice() == jpeg
    }
    pub fn observed_at(&self, captured: Instant) -> Self {
        Self {
            frame: Frame {
                jpeg: self.frame.jpeg.clone(),
                captured,
            },
            signature: self.signature.clone(),
        }
    }
}
#[derive(Default)]
pub struct FrameBuffer {
    frames: VecDeque<Frame>,
    signatures: VecDeque<Arc<[u8]>>,
}
impl FrameBuffer {
    #[cfg(test)]
    pub fn consider(
        &mut self,
        img: DynamicImage,
        threshold: u32,
        max_width: u32,
        quality: u8,
    ) -> Result<bool> {
        let scaled = if img.width() > max_width {
            img.resize(max_width, u32::MAX, FilterType::Triangle)
        } else {
            img
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode_image(&scaled.to_rgb8())?;
        let prepared = PreparedFrame::from_jpeg(jpeg, Instant::now())?;
        Ok(self.insert(prepared, threshold))
    }
    pub fn insert(&mut self, prepared: PreparedFrame, threshold: u32) -> bool {
        let PreparedFrame { frame, signature } = prepared;
        // Compare against every retained frame, not only the previous one.
        // A pixel must move at least 24 RGB levels before counting as changed.
        let duplicate = self.signatures.iter().rposition(|old| {
            let changed = old
                .chunks_exact(3)
                .zip(signature.chunks_exact(3))
                .filter(|(a, b)| {
                    a.iter()
                        .zip(b.iter())
                        .map(|(x, y)| x.abs_diff(*y) as u32)
                        .sum::<u32>()
                        >= 72
                })
                .count();
            changed * 100 < 64 * 36 * threshold as usize
        });
        if let Some(index) = duplicate {
            // Similar scenes share a history slot, never an old image. Small
            // text edits must still appear in the current screenshot.
            self.frames.remove(index);
            self.signatures.remove(index);
        }
        if self.frames.len() == 10 {
            self.frames.pop_front();
            self.signatures.pop_front();
        }
        self.frames.push_back(frame);
        self.signatures.push_back(signature);
        duplicate.is_none()
    }
    pub fn latest_five(&self) -> Vec<Frame> {
        self.frames
            .iter()
            .skip(self.frames.len().saturating_sub(5))
            .cloned()
            .collect()
    }
    pub fn stats(&self) -> (usize, usize) {
        (
            self.frames.len(),
            self.frames.iter().map(|f| f.jpeg.len()).sum(),
        )
    }
    pub fn latest_age_seconds(&self) -> Option<f64> {
        self.frames
            .back()
            .map(|f| f.captured.elapsed().as_secs_f64())
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.signatures.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    fn solid(n: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 60, Rgb([n, n, n])))
    }
    #[test]
    #[ignore = "Local CPU timing benchmark, not a correctness test"]
    fn capture_preparation_benchmark() {
        let image = RgbImage::from_fn(1280, 720, |x, y| {
            Rgb([
                (x % 256) as u8,
                (y % 256) as u8,
                if (x / 80 + y / 40) % 2 == 0 { 40 } else { 200 },
            ])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 60)
            .encode_image(&image)
            .unwrap();
        let started = Instant::now();
        for _ in 0..10 {
            let image = image::load_from_memory(&jpeg).unwrap();
            std::hint::black_box(image.resize_exact(64, 36, FilterType::Triangle).to_rgb8());
            let mut redundant = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut redundant, 60)
                .encode_image(&image.to_rgb8())
                .unwrap();
            std::hint::black_box(redundant);
        }
        let before = started.elapsed();
        let started = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(PreparedFrame::from_jpeg(jpeg.clone(), Instant::now()).unwrap());
        }
        let after = started.elapsed();
        let prepared = PreparedFrame::from_jpeg(jpeg, Instant::now()).unwrap();
        let started = Instant::now();
        for _ in 0..10000 {
            std::hint::black_box(prepared.observed_at(Instant::now()));
        }
        eprintln!(
            "1280x720 preparation: old {:.2} ms/frame; new {:.2} ms/frame; identical-screen cache {:.4} ms/frame",
            before.as_secs_f64() * 100.0,
            after.as_secs_f64() * 100.0,
            started.elapsed().as_secs_f64() * 0.1
        );
    }
    #[test]
    fn capture_bytes_are_not_recompressed_and_exact_repeats_share_memory() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 60)
            .encode_image(&solid(50).to_rgb8())
            .unwrap();
        let when = Instant::now();
        let prepared = PreparedFrame::from_jpeg(jpeg.clone(), when).unwrap();
        let mut b = FrameBuffer::default();
        b.insert(prepared.clone(), 12);
        let first = b.latest_five().pop().unwrap();
        assert_eq!(first.jpeg.as_slice(), jpeg.as_slice());
        assert_eq!(first.captured, when);
        assert!(prepared.matches(&jpeg));
        b.insert(
            prepared.observed_at(when + std::time::Duration::from_millis(250)),
            12,
        );
        let repeated = b.latest_five().pop().unwrap();
        assert!(Arc::ptr_eq(&first.jpeg, &repeated.jpeg));
        assert_eq!(b.stats().0, 1);
        assert!(PreparedFrame::from_jpeg(vec![0; 1_000_001], when).is_err());
        assert!(PreparedFrame::from_jpeg(b"not a JPEG".to_vec(), when).is_err());
    }
    #[test]
    fn rejects_duplicates_and_small_changes() {
        let mut b = FrameBuffer::default();
        assert!(b.consider(solid(0), 12, 1280, 60).unwrap());
        assert!(!b.consider(solid(10), 12, 1280, 60).unwrap());
        assert!(b.consider(solid(100), 12, 1280, 60).unwrap());
        assert!(!b.consider(solid(0), 12, 1280, 60).unwrap());
        assert_eq!(b.signatures.back().unwrap()[0], 0);
        assert_eq!(b.stats().0, 2);
    }
    #[test]
    fn eviction_and_order() {
        let mut b = FrameBuffer::default();
        // Unique spatial patterns survive the perceptual difference threshold.
        for i in 0..12 {
            let im = RgbImage::from_fn(128, 72, |x, y| {
                Rgb([if (((x / 16) * 13 + (y / 12) * 7 + i * 31) % 97) < 48 {
                    0
                } else {
                    255
                }; 3])
            });
            b.consider(DynamicImage::ImageRgb8(im), 1, 1280, 60)
                .unwrap();
        }
        assert_eq!(b.stats().0, 10);
        let last = b.latest_five();
        assert_eq!(last.len(), 5);
        assert!(last.windows(2).all(|x| x[0].captured <= x[1].captured));
        b.clear();
        assert_eq!(b.stats(), (0, 0));
    }
    #[test]
    fn similar_screens_replace_the_jpeg_not_just_its_age() {
        let mut b = FrameBuffer::default();
        b.consider(solid(0), 12, 1280, 90).unwrap();
        let old_bytes = Arc::downgrade(&b.frames.back().unwrap().jpeg);
        let mut current = RgbImage::from_pixel(100, 60, Rgb([0, 0, 0]));
        for y in 0..10 {
            for x in 0..10 {
                current.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        assert!(
            !b.consider(DynamicImage::ImageRgb8(current), 12, 1280, 90)
                .unwrap()
        );
        assert_eq!(b.stats().0, 1);
        assert!(
            old_bytes.upgrade().is_none(),
            "The obsolete JPEG must be released"
        );
        let newest = b.latest_five().pop().unwrap();
        let decoded = image::load_from_memory(&newest.jpeg).unwrap().to_rgb8();
        assert!(
            decoded.get_pixel(3, 3)[0] > 220,
            "The small screen edit must be visible"
        );
    }
    #[test]
    fn revisiting_a_scene_keeps_its_new_contents() {
        let mut b = FrameBuffer::default();
        b.consider(solid(0), 12, 1280, 90).unwrap();
        b.consider(solid(255), 12, 1280, 90).unwrap();
        assert!(!b.consider(solid(10), 12, 1280, 90).unwrap());
        assert_eq!(b.stats().0, 2);
        let frames = b.latest_five();
        let decoded = image::load_from_memory(&frames.last().unwrap().jpeg)
            .unwrap()
            .to_rgb8();
        assert!(
            decoded.get_pixel(40, 30)[0] >= 8,
            "Revisited scene must not reuse original black JPEG"
        );
        b.clear();
        assert!(b.latest_five().is_empty());
        assert!(b.latest_age_seconds().is_none());
    }
}
