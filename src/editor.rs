use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use anyhow::Error;
use crossbeam_channel::unbounded;
use hound::{WavSpec, SampleFormat, WavWriter};
use image::{DynamicImage, Rgba, RgbImage};
use image::imageops::{overlay, FilterType};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use lerp::Lerp;
use rodio::Source;
use rodio::source::UniformSourceIterator;
use threadpool::ThreadPool;


#[derive(Debug, Clone)]
pub struct EditorMeta {
    width: u32,
    height: u32,
    fps: f32,
    duration: Duration
}

impl EditorMeta {
    /// Get frame count
    pub fn frames(&self) -> usize {
        (self.fps * self.duration.as_secs_f32()) as usize
    }
}

/// Video editor
pub struct Editor {
    layers: Vec<Layer>,
    meta: EditorMeta,
}

impl Editor {
    /// Create new editor instance
    pub fn new(width: u32, height: u32, duration: Duration, fps: f32) -> Editor {
        Editor { layers: vec![], meta: EditorMeta { width, height, duration, fps } }
    }

    /// Add new layer
    pub fn layer(mut self, layer: Layer) -> Self {
        self.layers.push(layer);
        self
    }
}

/// Layer which can be overlayed over other layers in Editor
pub struct Layer {
    offset: Duration,
    effects: Vec<Box<dyn EditorEffect + Send + Sync>>,
    transform: Transform,
    duration: Duration,
    speed: f32,
    data: Box<dyn LayerData + Send + Sync>
}


impl Layer {
    /// Create new layer
    pub fn new(data: Box<dyn LayerData + Send + Sync + 'static>, offset: Duration, transform: Transform) -> Layer {
        Layer {
            duration: data.duration(),
            offset,
            data,
            transform, 
            speed: 1.0,
            effects: vec![]
        }
    }

    /// Add new effect
    pub fn effect(mut self, effect: impl EditorEffect + Send + Sync + 'static) -> Self {
        self.effects.push(Box::new(effect));
        self
    }

    /// Set new duration of this layer
    pub fn duration(mut self, duration: Duration) -> Self {
        if duration > self.data.duration() {
            self.duration = duration;
        } else {
            self.duration = duration;
        }
        self
    }

    /// Change the speed of this video
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Generate image from frame
    pub fn frame(&self, offset: Duration, base: &mut DynamicImage, meta: &EditorMeta) -> Result<(), Error> {
        let duration = Duration::from_secs_f32(self.duration.as_secs_f32() * self.speed);
        if offset < self.offset || offset > (duration + self.offset) {
            return Ok(())
        }
        let pos = Duration::from_secs_f32((offset - self.offset).as_secs_f32() * self.speed);
        if let Ok(Some(mut frame)) = self.data.frame(pos) {
            // Effects
            let mut transform = self.transform;
            for effect in &self.effects {
                frame = effect.apply_video_effect(frame, pos, duration, &mut transform,meta);
            }
            // Merge
            let (x, y) = transform.calculate(meta.width, meta.height);
            overlay(base, &frame, x, y);
        }
        return Ok(())
    }
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Pixels
    Px(i64, i64),
    /// 0 -> 1
    Percent(f32, f32)
}

impl Transform {
    /// (0, 0) transform
    pub const ZERO: Transform = Transform::Px(0, 0);

    /// Create new Pixel position
    pub fn px(x: i64, y: i64) -> Transform {
        Transform::Px(x, y)
    }

    /// New position in percentage relative to the screen
    pub fn percent(x: f32, y: f32) -> Transform {
        Transform::Percent(x, y)
    }

    /// Calculate position with width and height
    fn calculate(&self, width: u32, height: u32) -> (i64, i64) {
        match *self {
            Transform::Px(x, y) => (x, y),
            Transform::Percent(x, y) => ((width as f32 * x) as i64, (height as f32 * y) as i64),
        }
    }
}

pub trait EditorEffect {
    /// Apply video effect and return the frame
    fn apply_video_effect(&self, frame: DynamicImage, offset: Duration, duration: Duration, transform: &mut Transform, meta: &EditorMeta) -> DynamicImage;
    /// Apply audio effect and return mutated stream
    fn apply_audio_effect(&self, audio: AudioData) -> AudioData;
}

pub enum Effect {
    /// Resize to base size, force to ignore aspect ratio
    ScaleToBase { force: bool }, 
    Scale { x: f32, y: f32 },
    ScaleOverTime { x0: f32, y0: f32, x1: f32, y1: f32 },
    /// Angle in radians, uncropped = slow
    Rotate { angle: f32, uncropped: bool },
    /// Andle in radians, uncropped = slow
    RotateOverTime { a0: f32, a1: f32, uncropped: bool },
    MovePx { x: i64, y: i64 },
    AudioGain { gain: f32 },
}

impl EditorEffect for Effect {
    /// Apply an effect to frame
    fn apply_video_effect(&self, frame: DynamicImage, offset: Duration, duration: Duration, transform: &mut Transform, meta: &EditorMeta) -> DynamicImage {
        match self {
            // Scale it to base frame size
            Effect::ScaleToBase { force }=> {
                if *force {
                    frame.resize_exact(meta.width, meta.height, FilterType::Nearest)
                } else {
                    if frame.width() > meta.width || frame.height() > meta.height {
                        frame.resize(meta.width, meta.height, FilterType::Nearest)
                    } else { 
                        frame
                    }
                }
            },
            // Scale the frame
            Effect::Scale { x, y } => {
                let (w, h) = (frame.width() as f32 * x, frame.height() as f32 * y);
                frame.resize_exact(w as u32, h as u32, FilterType::Nearest)
            },
            // Scale the time over time
            Effect::ScaleOverTime { x0, y0, x1, y1 } => {
                let t = offset.as_secs_f32() / duration.as_secs_f32();
                let (w, h) = (frame.width() as f32 * x0.lerp(*x1, t), frame.height() as f32 * y0.lerp(*y1, t));
                frame.resize_exact(w as u32, h as u32, FilterType::Nearest)
            },
            // Rotate the frame
            Effect::Rotate { angle, uncropped } => {
                match *uncropped {
                    true => rotate_uncropped(&frame, *angle),
                    false => rotate_about_center(&frame.to_rgba8(), *angle, Interpolation::Nearest, Rgba([0, 0, 0, 0])).into()
                }
            },
            // Rotate the frame based on time
            Effect::RotateOverTime { a0, a1, uncropped } => {
                let t = offset.as_secs_f32() / duration.as_secs_f32();
                let a = a0.lerp(*a1, t);
                match *uncropped {
                    true => rotate_uncropped(&frame, a),
                    false => rotate_about_center(&frame.to_rgba8(), a, Interpolation::Nearest, Rgba([0, 0, 0, 0])).into()
                }
            },
            // Move by x, y
            Effect::MovePx { x, y } => {
                let position = transform.calculate(meta.width, meta.height);
                let t = offset.as_secs_f32() / duration.as_secs_f32();
                let (x, y) = ((position.0 as f32).lerp((position.0 + *x) as f32, t) as i64, (position.1 as f32).lerp((position.1 + *y) as f32, t) as i64);
                *transform = Transform::px(x, y);
                frame
            },
            // Audio effects
            Effect::AudioGain { .. } => frame
        }
    }

    /// Apply audio effect on source
    fn apply_audio_effect(&self, audio: AudioData) -> AudioData {
        match self {
            // Add gain to the audio
            Effect::AudioGain { gain } => {
                AudioData::new(audio.source.amplify(*gain))
            },

            // Video effects
            _ => audio
        }
    }
}

/// Data of layer
pub trait LayerData {
    /// Get duration of this layer
    fn duration(&self) -> Duration;
    /// Generate current frame
    fn frame(&self, offset: Duration) -> Result<Option<DynamicImage>, Error>;
    /// Get the layer's audio
    fn audio(&self) -> Result<Option<AudioData>, Error>;
}

/// Layer audio data
pub struct AudioData {
    source: Box<dyn Source<Item = f32> + Send + Sync>
}

impl AudioData {
    /// Source must be already in f32
    pub fn new(source: impl Source<Item = f32> + Send + Sync + 'static) -> AudioData {
        AudioData { source: Box::new(source) }
    }

    /// Make self uniform
    fn uniform(self, sample_rate: u32, channels: u16) -> Self {
        AudioData::new(UniformSourceIterator::new(self.source, channels, sample_rate))
    }

    /// Change speed of this audio
    /// WARNING: Call before uniform
    fn speed(self, speed: f32) -> Self {
        if speed == 1.0 {
            self
        } else {
            AudioData::new(self.source.speed(speed))
        }
    }
}


pub struct Renderer {
    editor: Editor, 
}

impl Renderer {
    /// Create new renderer instance
    pub fn new(editor: Editor) -> Renderer {
        Renderer { editor }
    }

    /// Frame count of final output
    pub fn frame_count(&self) -> usize {
        self.editor.meta.frames()
    }

    /// Render single frame
    pub fn render_frame(&self, frame_index: usize) -> Result<Option<DynamicImage>, Error> {
        if frame_index >= self.editor.meta.frames() {
            return Ok(None);
        }

        // Create base frame
        let base = RgbImage::from_raw(self.editor.meta.width, self.editor.meta.height, vec![0u8; self.editor.meta.width as usize * self.editor.meta.height as usize * 3]).unwrap();
        let mut base = DynamicImage::from(base);
        for layer in &self.editor.layers {
            layer.frame(Duration::from_secs_f32(frame_index as f32 / self.editor.meta.fps), &mut base, &self.editor.meta)?;
        }
        Ok(Some(base))
    }

    /// Render audio
    pub fn render_audio(&self, sample_rate: u32, channels: u16) -> Result<Vec<f32>, Error> {
        // Get sources
        let mut output = vec![];
        let duration = self.editor.meta.duration;
        let mut sample: usize = 0;
        let mut queue = self.editor.layers.iter().filter(|l| matches!(l.data.audio(), Ok(Some(_)))).collect::<Vec<_>>();
        let mut sources = vec![];
        // Iter over samples
        loop {
            // EOF
            let pos = Duration::from_secs_f32(sample as f32 / sample_rate as f32);
            if pos > duration  {
                break;
            }

            // Find audio source
            if !queue.is_empty() {
                let mut i = 0;
                loop {
                    if &queue[i].offset <= &pos {
                        // Make sure they're the same format
                        let layer = queue.remove(i);
                        let mut src = layer.data.audio()?.unwrap().speed(layer.speed).uniform(sample_rate, channels);
                        // Apply effects
                        for effect in &layer.effects {
                            src = effect.apply_audio_effect(src);
                        }
                        sources.push(src.source);
                    } else {
                        i += 1;
                    }
                    if i == queue.len() {
                        break;
                    }
                }
            }

            // Merge audio sources
            for _ in 0..channels {
                let mut sample = vec![];
                let mut new_sources = vec![];
                for mut source in sources {
                    match source.next() {
                        Some(s) => {
                            sample.push(s);
                            new_sources.push(source);
                        },
                        None => continue
                    }
                }
                // Average out the source
                sources = new_sources;
                if sample.is_empty() {
                    output.push(0.0)
                } else {
                    output.push(sample.iter().sum::<f32>()) // / sample.len() as f32                    
                }
            }
            sample += 1;
        }

        return Ok(output)
    }

    /// Render full video with multiple threads
    pub fn render_full_multithreaded(self: Arc<Self>, output: impl AsRef<Path>, threads: usize) -> Result<Arc<Self>, Error> {
        std::fs::create_dir_all(&output)?;
        let frame_count = self.frame_count();
        let (tx, rx) = unbounded();
        let pool = ThreadPool::new(threads);
        let output = Arc::new(output.as_ref().to_owned());
        // Start threadpool
        for i in 0..frame_count {
            let tx = tx.clone();
            let renderer = self.clone();
            let output = output.clone();
            pool.execute( move || {
                // Render the frame
                let render_frame = || -> Result<(), Error> {
                    let frame = renderer.render_frame(i)?.ok_or(anyhow!("Missing frame"))?;
                    frame.save(output.join(format!("{:06}.png", i+1)))?;
                    Ok(())
                };
                tx.send(render_frame()).ok();
                
            });
        }
        // Count the frames
        std::mem::drop(tx);
        let mut i = 0;
        for _ in rx {
            i += 1;
            if i % 50 == 0 {
                debug!("Done: {i} / {frame_count}");
            }
        }
        Ok(self)
    }

    /// Render audio to .wav
    pub fn render_audio_wav(&self, output: impl AsRef<Path>, sample_rate: u32, channels: u16) -> Result<(), Error> {
        let audio = self.render_audio(sample_rate, channels)?;
        let spec = WavSpec { channels, sample_rate, bits_per_sample: 32, sample_format: SampleFormat::Float };
        let mut writer = WavWriter::create(output, spec)?;
        for sample in audio {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        Ok(())
    }
}

/// Rotate uncropped (slow)
/// Modified version of: https://github.com/image-rs/imageproc/issues/323
fn rotate_uncropped(image: &DynamicImage, angle: f32) -> DynamicImage {
    // Calculate the size of the image
    let (new_width, new_height) = {
        let angle = PI / 4.0;
        let (width, height) = (image.width() as f32, image.height() as f32);
        (
            (width * angle.cos().abs() + height * angle.sin().abs()) as u32,
            (height * angle.cos().abs() + width * angle.sin().abs()) as u32,
        )
    };
    // Copy to new image
    let mut new_image = DynamicImage::new_rgba8(new_width, new_height).into_rgba8();
    let (offset_x, offset_y) = (new_width - image.width(), new_height - image.height());
    overlay(&mut new_image, image, offset_x as i64 / 2, offset_y as i64 / 2);
    // Rotate
    let output = rotate_about_center(&new_image, angle, Interpolation::Nearest, Rgba([0, 0, 0, 0u8]));
    output.into()
}