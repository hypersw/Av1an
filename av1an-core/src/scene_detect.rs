use ffmpeg_next::format::Pixel;
use smallvec::{smallvec, SmallVec};

use crate::Encoder;
use crate::{ffmpeg, into_smallvec, progress_bar, Input, ScenecutMethod, Verbosity};
use ansi_term::Style;
use av_scenechange::{detect_scene_changes, DetectionOptions, SceneDetectionSpeed};

use std::process::{Command, Stdio};

pub fn av_scenechange_detect(
  input: &Input,
  encoder: Encoder,
  total_frames: usize,
  min_scene_len: usize,
  verbosity: Verbosity,
  sc_pix_format: Option<Pixel>,
  sc_method: ScenecutMethod,
  sc_downscale_height: Option<usize>,
) -> anyhow::Result<Vec<usize>> {
  if verbosity != Verbosity::Quiet {
    eprintln!("{}", Style::default().bold().paint("Scene detection"));
    progress_bar::init_progress_bar(total_frames as u64);
  }

  let mut frames = scene_detect(
    input,
    encoder,
    if verbosity == Verbosity::Quiet {
      None
    } else {
      Some(Box::new(|frames, _keyframes| {
        progress_bar::set_pos(frames as u64);
      }))
    },
    min_scene_len,
    sc_pix_format,
    sc_method,
    sc_downscale_height,
  )?;

  progress_bar::finish_progress_bar();

  if frames[0] == 0 {
    // TODO refactor the chunk creation to not require this
    // Currently, this is required for compatibility with create_video_queue_vs
    frames.remove(0);
  }

  Ok(frames)
}

/// Detect scene changes using rav1e scene detector.
pub fn scene_detect(
  input: &Input,
  encoder: Encoder,
  callback: Option<Box<dyn Fn(usize, usize)>>,
  min_scene_len: usize,
  sc_pix_format: Option<Pixel>,
  sc_method: ScenecutMethod,
  sc_downscale_height: Option<usize>,
) -> anyhow::Result<Vec<usize>> {
  let bit_depth;

  let filters: SmallVec<[String; 4]> = match (sc_downscale_height, sc_pix_format) {
    (Some(sdh), Some(spf)) => into_smallvec![
      "-vf",
      format!(
        "format={},scale=-2:'min({},ih)'",
        spf.descriptor().unwrap().name(),
        sdh
      )
    ],
    (Some(sdh), None) => into_smallvec!["-vf", format!("scale=-2:'min({},ih)'", sdh)],
    (None, Some(spf)) => into_smallvec!["-pix_fmt", spf.descriptor().unwrap().name()],
    (None, None) => smallvec![],
  };

  let decoder = &mut y4m::Decoder::new(match input {
    Input::VapourSynth(path) => {
      bit_depth = crate::vapoursynth::bit_depth(path.as_ref())?;
      let vspipe = Command::new("vspipe")
        .arg("-y")
        .arg(path)
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?
        .stdout
        .unwrap();

      if !filters.is_empty() {
        Command::new("ffmpeg")
          .stdin(vspipe)
          .args(["-i", "pipe:", "-f", "yuv4mpegpipe", "-strict", "-1"])
          .args(filters)
          .arg("-")
          .stdout(Stdio::piped())
          .stderr(Stdio::null())
          .spawn()?
          .stdout
          .unwrap()
      } else {
        vspipe
      }
    }
    Input::Video(path) => {
      let input_pix_format = ffmpeg::get_pixel_format(path.as_ref())
        .unwrap_or_else(|e| panic!("FFmpeg failed to get pixel format for input video: {:?}", e));
      bit_depth = encoder.get_format_bit_depth(sc_pix_format.unwrap_or(input_pix_format))?;
      Command::new("ffmpeg")
        .args(["-r", "1", "-i"])
        .arg(path)
        .args(filters.as_ref())
        .args(["-f", "yuv4mpegpipe", "-strict", "-1", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?
        .stdout
        .unwrap()
    }
  })?;

  let options = DetectionOptions {
    min_scenecut_distance: Some(min_scene_len),
    analysis_speed: match sc_method {
      ScenecutMethod::Fast => SceneDetectionSpeed::Fast,
      ScenecutMethod::Standard => SceneDetectionSpeed::Standard,
    },
    ..DetectionOptions::default()
  };
  Ok(if bit_depth > 8 {
    detect_scene_changes::<_, u16>(decoder, options, callback).scene_changes
  } else {
    detect_scene_changes::<_, u8>(decoder, options, callback).scene_changes
  })
}
