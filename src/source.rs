use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::path::{PathBuf, Path};
use std::time::Duration;
use anyhow::Error;
use image::DynamicImage;
use rodio::source::SamplesConverter;
use rodio::{Decoder, Source};
use serde::{Serialize, Deserialize};
use image::io::Reader as ImageReader;

use crate::editor::{LayerData, AudioData};
use crate::ffmpeg::FFmpeg;

/// Loads and decodes files
pub struct FileLoader {
    video_cache_path: PathBuf,
    ffmpeg: FFmpeg
}

impl FileLoader {
    /// Create new instance
    pub fn new(video_cache_path: impl AsRef<Path>, ffmpeg: FFmpeg) -> FileLoader {
        FileLoader { video_cache_path: video_cache_path.as_ref().to_owned(), ffmpeg }
    }

    /// Load file from path by extension
    pub fn load_file(&self, path: impl AsRef<Path>) -> Result<MediaSource, Error> {
        let ext = path.as_ref().extension().ok_or(anyhow!("Missing extension"))?.to_string_lossy().to_ascii_lowercase();
        match &ext[..] {
            "jpg" | "jpeg" | "png" | "tiff" | "bmp" | "webp" => {
                Ok(MediaSource::Image(Image::new(path)))
            },
            "mp3" | "wav" | "ogg" | "flac" => {
                Ok(MediaSource::Audio(Audio::new(path)?))
            },
            "mp4" | "mov" | "wmv" | "avi" | "webm" | "gif" | "mkv" | "m4v" => {
                let filename = path.as_ref().file_name().unwrap().to_string_lossy();
                let filename = filename.split(".").next().unwrap().to_owned();
                Ok(MediaSource::Video(Video::load_or_cache(
                    path, 
                    self.video_cache_path.join(filename), 
                    &self.ffmpeg, 
                    [], 
                    []
                )?))
            },
            _ => Err(anyhow!("Unsupported extension"))
        }
    }
}

/// One of media sources
#[derive(Debug, Clone)]
pub enum MediaSource {
    Video(Video),
    Audio(Audio),
    Image(Image)
}

impl MediaSource {
    /// Get layer data
    pub fn layer_data_with_image_duration(self, duration: Duration) -> Result<Box<dyn LayerData + Send + Sync>, Error> {
        match self {
            MediaSource::Video(v) => return Ok(Box::new(VideoLayer::new(v))),
            MediaSource::Audio(a) => return Ok(Box::new(AudioLayer::new(a))),
            MediaSource::Image(i) => return Ok(Box::new(ImageLayer::new(&i, duration)?)),
        }
    }

    /// Get layer data with default image duration
    pub fn layer_data(self) -> Result<Box<dyn LayerData + Send + Sync>, Error> {
        self.layer_data_with_image_duration(Duration::from_secs(5))
    }
}

/// Metadata of video
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMeta {
    pub width: u32,
    pub height: u32,
    pub duration: Duration,
    pub frames: usize
}

/// Video source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub path: PathBuf, 
    pub audio: bool,
    pub meta: VideoMeta,
}

impl Video {
    /// Cache the given video
    pub fn load_or_cache<'a, A, B>(
        input_path: impl AsRef<Path>, 
        cache_path: impl AsRef<Path>, 
        ffmpeg: &FFmpeg, 
        ffmpeg_split_args: A,
        ffmpeg_audio_args: B
    ) -> Result<Video, Error> 
    where 
        A: IntoIterator<Item = &'a OsStr>,
        B: IntoIterator<Item = &'a OsStr>
    {
        let out_path = cache_path.as_ref();
        let meta_path = out_path.join("meta.json");
        // Check if exists
        match std::fs::read_to_string(&meta_path) {
            Ok(data) => {
                return Ok(serde_json::from_str(&data)?);
            },
            // Cache
            Err(_) => { info!("Caching video: {:?}", input_path.as_ref()) },
        }
        
        // Split
        let meta = ffmpeg.video_meta(&input_path)?;
        std::fs::create_dir_all(&out_path.join("frames"))?;
        ffmpeg.convert(&input_path, out_path.join("frames").join("%06d.png"), ffmpeg_split_args)?;
        let audio = ffmpeg.convert(&input_path.as_ref(), &out_path.join("audio.mp3"), ffmpeg_audio_args).is_ok();
        // Generate meta
        let video = Video {
            meta,
            audio,
            path: out_path.to_owned(),
        };
        // Save meta
        std::fs::write(&meta_path, serde_json::to_string_pretty(&video)?)?;
        Ok(video)
    }

    /// Load frame as image
    pub fn frame(&self, index: usize) -> Result<DynamicImage, Error> {
        let image = ImageReader::open(self.path.join("frames").join(format!("{:06}.png", index+1)))?.decode()?;
        Ok(image)
    }

    /// Get audio of this video
    pub fn audio(&self) -> Option<Result<Audio, Error>> {
        match self.audio {
            true => Some(Audio::new(self.path.join("audio.mp3"))),
            false => None
        }
    }

    /// Delete own cache
    pub fn uncache(self) -> Result<(), Error> {
        std::fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}

/// Image source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub path: PathBuf
}

impl Image {
    pub fn new(path: impl AsRef<Path>) -> Image {
        Image { path: path.as_ref().into() }
    }

    /// Load this image
    pub fn load(&self) -> Result<DynamicImage, Error> {
        let i = ImageReader::open(&self.path)?.decode()?;
        Ok(i)
    }
}

/// Audio source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audio {
    path: PathBuf,
    duration: Duration,
}

impl Audio {
    /// Create new audio source
    pub fn new(path: impl AsRef<Path>) -> Result<Audio, Error> {
        let mut audio = Audio { path: path.as_ref().into(), duration: Duration::ZERO };
        audio.duration = audio.source()?.total_duration().unwrap_or(Duration::ZERO);
        Ok(audio)
    }

    /// Get rodio source
    pub fn source(&self) -> Result<Decoder<BufReader<File>>, Error> {
        let reader = BufReader::new(File::open(&self.path)?);
        let source = Decoder::new(reader)?;
        Ok(source)
    }

    /// Get audio duration
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// Still image layer
pub struct ImageLayer {
    image: DynamicImage,
    duration: Duration
}

impl ImageLayer {
    /// Create new imagelayer
    pub fn new(image: &Image, duration: Duration) -> Result<ImageLayer, Error> {
        Ok(ImageLayer { image: image.load()?, duration })
    }
}

impl LayerData for ImageLayer {
    fn duration(&self) -> Duration {
        self.duration
    }

    fn frame(&self, _offset: Duration) -> Result<Option<DynamicImage>, Error>  {
        Ok(Some(self.image.clone()))
    }

    fn audio(&self) -> Result<Option<AudioData>, Error> {
        Ok(None)
    }
}

/// Video layer
pub struct VideoLayer {
    video: Video
}

impl VideoLayer {
    /// Create new video layer
    pub fn new(video: Video) -> VideoLayer {
        VideoLayer { video }
    }
}

impl LayerData for VideoLayer {
    fn duration(&self) -> Duration {
        self.video.meta.duration
    }

    fn frame(&self, offset: Duration) -> Result<Option<DynamicImage>, Error> {
        let t = offset.as_secs_f32() / self.video.meta.duration.as_secs_f32();
        let frame = (t * self.video.meta.frames as f32) as usize;
        Ok(Some(self.video.frame(frame)?))
    }

    fn audio(&self) -> Result<Option<AudioData>, Error> {
        match self.video.audio() {
            Some(audio) => Ok(Some(AudioData::new(SamplesConverter::new(audio?.source()?)))),
            None => Ok(None)
        }
    }
}

/// Audio layer
pub struct AudioLayer {
    audio: Audio
}

impl AudioLayer {
    /// Create new audio layer
    pub fn new(audio: Audio) -> AudioLayer {
        AudioLayer { audio }
    }
}

impl LayerData for AudioLayer {
    fn duration(&self) -> Duration {
        self.audio.duration()
    }

    fn frame(&self, _offset: Duration) -> Result<Option<DynamicImage>, Error> {
        Ok(None)
    }

    fn audio(&self) -> Result<Option<AudioData>, Error> {
        Ok(Some(AudioData::new(SamplesConverter::new(self.audio.source()?))))
    }
}