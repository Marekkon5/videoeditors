use std::path::Path;
use std::process::{Command, Child, Stdio};
use std::ffi::OsStr;
use std::time::Duration;
use anyhow::Error;

use crate::source::VideoMeta;

/// Wait for ffmpeg output
fn wait_output(child: Child) -> Result<(), Error> {
    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        info!("ffmpeg stderr: {stderr}");
    }
    if !output.status.success() {
        return Err(anyhow!("ffmpeg failed: {stderr}"));
    }
    Ok(())
}


#[derive(Debug, Clone)]
pub struct FFmpeg {
    ffmpeg: String,
    ffprobe: String
}

impl FFmpeg {
    /// Create new instance with custom ffmpeg & ffprobe binary paths
    pub fn new(ffmpeg_bin: &str, ffprobe_bin: &str) -> FFmpeg {
        FFmpeg { ffmpeg: ffmpeg_bin.to_string(), ffprobe: ffprobe_bin.to_string() }
    }

    /// Create base ffmpeg command with no logging and piped stdio
    fn ffmpeg(&self, stdin: bool, stdout: bool) -> Command {
        let mut c = Command::new(&self.ffmpeg);
        c.args(["-y", "-hide_banner", "-loglevel", "error"]);
        if stdin {
            c.stdin(Stdio::piped());
        }
        if stdout {
            c.stdout(Stdio::piped());
        }
        c
    }

    /// Basic ffmpeg convert command
    pub fn convert<A, O>(&self, path: impl AsRef<Path>, output: impl AsRef<Path>, args: A) -> Result<(), Error> 
    where
        A: IntoIterator<Item = O>,
        O: AsRef<OsStr> 
    {
        let child = self.ffmpeg(false, false)
            .arg("-i").arg(path.as_ref().as_os_str())
            .args(args)
            .arg(output.as_ref().as_os_str())
            .spawn()?;
        wait_output(child)?;
        Ok(())
    }

    /// Use ffprobe to get video metadata
    pub fn video_meta(&self, path: impl AsRef<Path>) -> Result<VideoMeta, Error> {
        let output = String::from_utf8_lossy(&Command::new(&self.ffprobe)
            .args(["-v", "error", "-select_streams", "v:0", "-count_frames", "-show_entries", "stream=width,height,duration,nb_read_frames", "-of", "csv=p=0"])
            .arg(path.as_ref().as_os_str())
            .output()?
            .stdout
        ).to_string();
        // Parse 
        let mut i = output.trim().split(",");
        let meta = VideoMeta {
            width: i.next().ok_or(anyhow!("Missing width"))?.parse()?,
            height: i.next().ok_or(anyhow!("Missing height"))?.parse()?,
            duration: Duration::from_secs_f32(i.next().ok_or(anyhow!("Missing duration"))?.parse()?),
            frames: i.next().ok_or(anyhow!("Missing frame count"))?.parse()?
        };
        Ok(meta)
    }
}

impl Default for FFmpeg {
    fn default() -> Self {
        FFmpeg { ffmpeg: "ffmpeg".to_string(), ffprobe: "ffprobe".to_string() }     
    }
}




