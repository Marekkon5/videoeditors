use std::error::Error;
use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use videoeditor::{FFmpeg, FileLoader, Editor};
use videoeditor::editor::{Layer, Transform, Effect, Renderer};

fn main() {
    std::env::set_var("RUST_LOG", "debug");
    pretty_env_logger::init();

    run().expect("Failed");
}

fn run() -> Result<(), Box<dyn Error>> {
    let video_cache = "/tmp/video_cache";
    let output = Path::new("/tmp/output");

    // Use ffmpeg::new(ffmpeg, ffprobe) if you wish to change the default binary path
    let ffmpeg = FFmpeg::default();
    let loader = FileLoader::new(video_cache, Duration::from_secs(3), ffmpeg.clone());
    let editor = Editor::new(640, 360, Duration::from_secs(10), 25.0);

    // Add layers
    let editor = editor
    // Base video layer
    .layer(
        Layer::new(
            loader.load_file("assets/sample.m4v")?,
            Duration::ZERO,
            Transform::ZERO
        )
        .effect(Effect::ScaleToBase { force: true })
    )
    // Sample image overlay
    .layer(
        Layer::new(
            loader.load_file("assets/sample.png")?,
            Duration::from_secs(5),
            Transform::Percent(0.5, 0.5)
        )
        .effect(Effect::ScaleOverTime { x0: 1.0, y0: 1.0, x1: 2.0, y1: 2.0 })
        .effect(Effect::RotateOverTime { a0: 0.0, a1: PI, uncropped: true })
    )
    // Add audio
    .layer(
        Layer::new(
            loader.load_file("assets/sample.mp3")?,
            Duration::from_secs(5),
            Transform::ZERO
        )
        .speed(0.5)
        .effect(Effect::AudioGain { gain: 0.5 })
    );

    // Render
    let renderer = Arc::new(Renderer::new(editor));
    let renderer = renderer.render_full_multithreaded(output.join("frames"), num_cpus::get())?;
    renderer.render_audio_wav(output.join("audio.wav"), 44100, 2)?;
    
    // Merge
    ffmpeg.convert(output.join("frames").join("%06d.png"), output.join("output.mp4"), [
        // Add audio
        "-i", &output.join("audio.wav").to_string_lossy().to_string(),
        // Video encoding parameters
        "-c:v", "libx264", "-vf", "fps=25", "-pix_fmt", "yuv420p", "-b:v", "600k",
        // Audio encoding parameters
        "-b:a", "128k", "-c:a", "aac", "-ar", "44100",
        // Streaming
        "-movflags", "+faststart"
    ])?;

    // Clean temp
    std::fs::remove_dir_all(output.join("frames"))?;
    std::fs::remove_file(output.join("audio.wav"))?;

    log::info!("Saved in: {:?}", output.join("output.mp4"));
    Ok(())
}