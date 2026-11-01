use anyhow::Result;
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use std::{collections::VecDeque, sync::Arc, time::Instant};

#[derive(Clone)]
pub struct Frame {
    pub jpeg: Arc<Vec<u8>>,
    pub captured: Instant,
}
#[derive(Default)]
pub struct FrameBuffer {
    frames: VecDeque<Frame>,
    signatures: VecDeque<Vec<u8>>,
}
impl FrameBuffer {
    pub fn consider(
        &mut self,
        img: DynamicImage,
        threshold: u32,
        max_width: u32,
        quality: u8,
    ) -> Result<bool> {
        let signature = img
            .resize_exact(64, 36, FilterType::Triangle)
            .to_rgb8()
            .into_raw();
        // Compare against every retained frame, not only the previous one.
        // A pixel must move at least 24 RGB levels before counting as changed.
        let duplicate = self.signatures.iter().position(|old| {
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
            // Revisiting an older scene must make it the newest context again.
            let mut frame = self.frames.remove(index).unwrap();
            frame.captured = Instant::now();
            self.frames.push_back(frame);
            let sig = self.signatures.remove(index).unwrap();
            self.signatures.push_back(sig);
            return Ok(false);
        }
        let (w, h) = img.dimensions();
        let scaled = if w > max_width {
            img.resize(
                max_width,
                ((h as u64 * max_width as u64 / w as u64) as u32).max(1),
                FilterType::Triangle,
            )
        } else {
            img
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode_image(&scaled.to_rgb8())?;
        // Hard bound keeps unusual/noisy screens from taking excessive space.
        anyhow::ensure!(
            jpeg.len() <= 1_000_000,
            "Compressed screenshot exceeds 1 MB; lower image width or quality"
        );
        if self.frames.len() == 10 {
            self.frames.pop_front();
            self.signatures.pop_front();
        }
        self.frames.push_back(Frame {
            jpeg: Arc::new(jpeg),
            captured: Instant::now(),
        });
        self.signatures.push_back(signature);
        Ok(true)
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
}
