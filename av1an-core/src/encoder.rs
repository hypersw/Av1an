use crate::{ffmpeg::compose_ffmpeg_pipe, inplace_vec, into_vec, list_index};
use cfg_if::cfg_if;
use ffmpeg_next::format::Pixel;
use itertools::chain;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, cmp, fmt::Display, path::PathBuf};
use thiserror::Error;

use std::iter::Iterator;

const NULL: &str = if cfg!(windows) { "nul" } else { "/dev/null" };

#[allow(non_camel_case_types)]
#[derive(
  Clone, Copy, PartialEq, Serialize, Deserialize, Debug, strum::EnumString, strum::IntoStaticStr,
)]
pub enum Encoder {
  aom,
  rav1e,
  vpx,
  #[strum(serialize = "svt-av1")]
  svt_av1,
  x264,
  x265,
}

impl Display for Encoder {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(<&'static str>::from(self))
  }
}

impl Encoder {
  /// Composes 1st pass command for 1 pass encoding
  pub fn compose_1_1_pass(self, params: Vec<String>, output: String) -> Vec<String> {
    match self {
      Self::aom => chain!(
        into_vec!["aomenc", "--passes=1"],
        params,
        into_vec!["-o", output, "-"],
      )
      .collect(),
      Self::rav1e => chain!(
        into_vec!["rav1e", "-", "-y"],
        params,
        into_vec!["--output", output]
      )
      .collect(),
      Self::vpx => chain!(
        into_vec!["vpxenc", "--passes=1"],
        params,
        into_vec!["-o", output, "-"]
      )
      .collect(),
      Self::svt_av1 => chain!(
        into_vec!["SvtAv1EncApp", "-i", "stdin", "--progress", "2"],
        params,
        into_vec!["-b", output],
      )
      .collect(),
      Self::x264 => chain!(
        into_vec![
          "x264",
          "--stitchable",
          "--log-level",
          "error",
          "--demuxer",
          "y4m",
        ],
        params,
        into_vec!["-", "-o", output]
      )
      .collect(),
      Self::x265 => chain!(
        into_vec!["x265", "--y4m"],
        params,
        into_vec!["-", "-o", output]
      )
      .collect(),
    }
  }

  /// Composes 1st pass command for 2 pass encoding
  pub fn compose_1_2_pass(self, params: Vec<String>, fpf: &str) -> Vec<String> {
    match self {
      Self::aom => chain!(
        into_vec!["aomenc", "--passes=2", "--pass=1"],
        params,
        into_vec![format!("--fpf={}.log", fpf), "-o", NULL, "-"],
      )
      .collect(),
      Self::rav1e => chain!(
        into_vec!["rav1e", "-", "-y", "-q"],
        params,
        into_vec!["--first-pass", format!("{}.stat", fpf), "--output", NULL]
      )
      .collect(),
      Self::vpx => chain!(
        into_vec!["vpxenc", "--passes=2", "--pass=1"],
        params,
        into_vec![format!("--fpf={}.log", fpf), "-o", NULL, "-"],
      )
      .collect(),
      Self::svt_av1 => chain!(
        into_vec![
          "SvtAv1EncApp",
          "-i",
          "stdin",
          "--progress",
          "2",
          "--irefresh-type",
          "2",
        ],
        params,
        into_vec![
          "--pass",
          "1",
          "--stats",
          format!("{}.stat", fpf),
          "-b",
          NULL,
        ],
      )
      .collect(),
      Self::x264 => chain!(
        into_vec![
          "x264",
          "--stitchable",
          "--log-level",
          "error",
          "--pass",
          "1",
          "--demuxer",
          "y4m",
        ],
        params,
        into_vec!["--stats", format!("{}.log", fpf), "-", "-o", NULL]
      )
      .collect(),
      Self::x265 => chain!(
        into_vec![
          "x265",
          "--stitchable",
          "--log-level",
          "error",
          "--pass",
          "1",
          "--demuxer",
          "y4m",
        ],
        params,
        into_vec!["--stats", format!("{}.log", fpf), "-", "-o", NULL]
      )
      .collect(),
    }
  }

  /// Composes 2st pass command for 2 pass encoding
  pub fn compose_2_2_pass(self, params: Vec<String>, fpf: &str, output: String) -> Vec<String> {
    match self {
      Self::aom => chain!(
        into_vec!["aomenc", "--passes=2", "--pass=2"],
        params,
        into_vec![format!("--fpf={}.log", fpf), "-o", output, "-"],
      )
      .collect(),
      Self::rav1e => chain!(
        into_vec!["rav1e", "-", "-y", "-q"],
        params,
        into_vec!["--second-pass", format!("{}.stat", fpf), "--output", output]
      )
      .collect(),
      Self::vpx => chain!(
        into_vec!["vpxenc", "--passes=2", "--pass=2"],
        params,
        into_vec![format!("--fpf={}.log", fpf), "-o", output, "-"],
      )
      .collect(),
      Self::svt_av1 => chain!(
        into_vec![
          "SvtAv1EncApp",
          "-i",
          "stdin",
          "--progress",
          "2",
          "--irefresh-type",
          "2",
        ],
        params,
        into_vec![
          "--pass",
          "2",
          "--stats",
          format!("{}.stat", fpf),
          "-b",
          output,
        ],
      )
      .collect(),
      Self::x264 => chain!(
        into_vec![
          "x264",
          "--stitchable",
          "--log-level",
          "error",
          "--pass",
          "2",
          "--demuxer",
          "y4m",
        ],
        params,
        into_vec!["--stats", format!("{}.log", fpf), "-", "-o", output]
      )
      .collect(),
      Self::x265 => chain!(
        into_vec![
          "x265",
          "--stitchable",
          "--log-level",
          "error",
          "--pass",
          "2",
          "--demuxer",
          "y4m",
        ],
        params,
        into_vec!["--stats", format!("{}.log", fpf), "-", "-o", output]
      )
      .collect(),
    }
  }

  /// Returns default settings for the encoder
  pub fn get_default_arguments(self) -> Vec<String> {
    match self {
      // aomenc automatically infers the correct bit depth, and thus for aomenc, not specifying
      // the bit depth is actually more accurate because if for example you specify
      // `--pix-format yuv420p`, aomenc will encode 10-bit when that is not actually the desired
      // pixel format.
      Encoder::aom => into_vec![
        "--threads=8",
        "--cpu-used=6",
        "--end-usage=q",
        "--cq-level=30",
        "--tile-columns=2",
        "--tile-rows=1",
        "--kf-max-dist=240",
        "--kf-min-dist=12",
      ],
      Encoder::rav1e => into_vec![
        "--tiles",
        "8",
        "--speed",
        "6",
        "--quantizer",
        "100",
        "--no-scene-detection",
      ],
      // vpxenc does not infer the pixel format from the input, so `-b 10` is still required
      // to work with the default pixel format (yuv420p10le).
      Encoder::vpx => into_vec![
        "--codec=vp9",
        "-b",
        "10",
        "--profile=2",
        "--threads=4",
        "--cpu-used=2",
        "--end-usage=q",
        "--cq-level=30",
        "--kf-max-dist=240",
        "--row-mt=1",
        "--auto-alt-ref=6",
      ],
      Encoder::svt_av1 => into_vec!["--preset", "4", "--keyint", "240", "--rc", "0", "--crf", "25"],
      Encoder::x264 => into_vec!["--preset", "slow", "--crf", "25"],
      Encoder::x265 => into_vec!["-p", "slow", "--crf", "25", "-D", "10"],
    }
  }

  /// Return number of default passes for encoder
  pub const fn get_default_pass(self) -> u8 {
    match self {
      Self::aom | Self::vpx => 2,
      _ => 1,
    }
  }

  /// Default quantizer range target quality mode
  pub const fn get_default_cq_range(self) -> (usize, usize) {
    match self {
      Self::aom | Self::vpx => (15, 55),
      Self::rav1e => (50, 140),
      Self::svt_av1 => (15, 50),
      Self::x264 | Self::x265 => (15, 35),
    }
  }

  /// Returns help command for encoder
  pub const fn help_command(self) -> [&'static str; 2] {
    match self {
      Self::aom => ["aomenc", "--help"],
      Self::rav1e => ["rav1e", "--fullhelp"],
      Self::vpx => ["vpxenc", "--help"],
      Self::svt_av1 => ["SvtAv1EncApp", "--help"],
      Self::x264 => ["x264", "--fullhelp"],
      Self::x265 => ["x265", "--fullhelp"],
    }
  }

  /// Get the name of the executable/binary for the encoder
  pub const fn bin(self) -> &'static str {
    match self {
      Self::aom => "aomenc",
      Self::rav1e => "rav1e",
      Self::vpx => "vpxenc",
      Self::svt_av1 => "SvtAv1EncApp",
      Self::x264 => "x264",
      Self::x265 => "x265",
    }
  }

  /// Get the default output extension for the encoder
  pub const fn output_extension(&self) -> &'static str {
    match &self {
      Self::aom | Self::rav1e | Self::vpx | Self::svt_av1 => "ivf",
      Self::x264 | Self::x265 => "mkv",
    }
  }

  /// Returns function pointer used for matching Q/CRF arguments in command line
  fn q_match_fn(self) -> fn(&str) -> bool {
    match self {
      Self::aom | Self::vpx => |p| p.starts_with("--cq-level="),
      Self::rav1e => |p| p == "--quantizer",
      Self::svt_av1 => |p| matches!(p, "--qp" | "-q" | "--crf"),
      Self::x264 | Self::x265 => |p| p == "--crf",
    }
  }

  fn replace_q(self, index: usize, q: usize) -> (usize, String) {
    match self {
      Self::aom | Self::vpx => (index, format!("--cq-level={}", q)),
      Self::rav1e | Self::svt_av1 | Self::x265 | Self::x264 => (index + 1, format!("{}", q)),
    }
  }

  /// Returns changed q/crf in command line arguments
  pub fn man_command(self, params: Vec<String>, q: usize) -> Vec<String> {
    let index = list_index(&params, self.q_match_fn())
      .unwrap_or_else(|| panic!("No match found for params: {:#?}", params));

    let mut new_params = params;
    let (replace_index, replace_q) = self.replace_q(index, q);
    new_params[replace_index] = replace_q;

    new_params
  }

  /// Parses the number of encoded frames
  ///
  /// # Safety
  ///
  /// The caller should not attempt to read the contents of `line` after
  /// this function has been called.
  pub(crate) unsafe fn parse_encoded_frames(self, line: &mut str) -> Option<u64> {
    use crate::parse::*;

    match self {
      Self::aom | Self::vpx => {
        cfg_if! {
          if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
            if is_x86_feature_detected!("sse4.1") && is_x86_feature_detected!("ssse3") {
              return parse_aom_vpx_frames_sse41(line.as_bytes_mut());
            }
          }
        }

        parse_aom_vpx_frames(line)
      }
      Self::rav1e => parse_rav1e_frames(line),
      Self::svt_av1 => parse_svt_av1_frames(line),
      Self::x264 | Self::x265 => parse_x26x_frames(line),
    }
  }

  /// Returns command used for target quality probing
  pub fn construct_target_quality_command(
    self,
    threads: usize,
    q: usize,
  ) -> Vec<Cow<'static, str>> {
    match &self {
      Self::aom => inplace_vec![
        "aomenc",
        "--passes=1",
        format!("--threads={}", threads),
        "--tile-columns=2",
        "--tile-rows=1",
        "--end-usage=q",
        "-b",
        "8",
        "--cpu-used=6",
        format!("--cq-level={}", q),
        "--enable-filter-intra=0",
        "--enable-smooth-intra=0",
        "--enable-paeth-intra=0",
        "--enable-cfl-intra=0",
        "--enable-obmc=0",
        "--enable-palette=0",
        "--enable-overlay=0",
        "--enable-intrabc=0",
        "--enable-angle-delta=0",
        "--reduced-tx-type-set=1",
        "--enable-dual-filter=0",
        "--enable-intra-edge-filter=0",
        "--enable-order-hint=0",
        "--enable-flip-idtx=0",
        "--enable-dist-wtd-comp=0",
        "--enable-interintra-wedge=0",
        "--enable-onesided-comp=0",
        "--enable-interintra-comp=0",
        "--enable-global-motion=0",
        "--enable-cdef=0",
        "--max-reference-frames=3",
        "--cdf-update-mode=2",
        "--deltaq-mode=0",
        "--enable-tpl-model=0",
        "--sb-size=64",
        "--min-partition-size=32",
        "--max-partition-size=32",
        "--kf-min-dist=12",
      ],
      Self::rav1e => inplace_vec![
        "rav1e",
        "-y",
        "-s",
        "10",
        "--threads",
        format!("{}", threads),
        "--tiles",
        "16",
        "--quantizer",
        format!("{}", q),
        "--low-latency",
        "--rdo-lookahead-frames",
        "5",
        "--no-scene-detection",
      ],
      Self::vpx => inplace_vec![
        "vpxenc",
        "-b",
        "10",
        "--profile=2",
        "--passes=1",
        "--pass=1",
        "--codec=vp9",
        format!("--threads={}", threads),
        "--cpu-used=9",
        "--end-usage=q",
        format!("--cq-level={}", q),
        "--row-mt=1",
      ],
      Self::svt_av1 => inplace_vec![
        "SvtAv1EncApp",
        "-i",
        "stdin",
        "--lp",
        format!("{}", threads),
        "--preset",
        "8",
        "--keyint",
        "240",
        "--crf",
        format!("{}", q),
        "--tile-rows",
        "1",
        "--tile-columns",
        "2",
        "--pred-struct",
        "0",
        "--sg-filter-mode",
        "0",
        "--enable-restoration-filtering",
        "0",
        "--cdef-level",
        "0",
        "--disable-dlf",
        "0",
        "--mrp-level",
        "0",
        "--enable-mfmv",
        "0",
        "--enable-local-warp",
        "0",
        "--enable-global-motion",
        "0",
        "--enable-interintra-comp",
        "0",
        "--obmc-level",
        "0",
        "--rdoq-level",
        "0",
        "--filter-intra-level",
        "0",
        "--enable-intra-edge-filter",
        "0",
        "--enable-pic-based-rate-est",
        "0",
        "--pred-me",
        "0",
        "--bipred-3x3",
        "0",
        "--compound",
        "0",
        "--ext-block",
        "0",
        "--hbd-md",
        "0",
        "--palette-level",
        "0",
        "--umv",
        "0",
        "--tf-level",
        "3",
      ],
      Self::x264 => inplace_vec![
        "x264",
        "--log-level",
        "error",
        "--demuxer",
        "y4m",
        "-",
        "--no-progress",
        "--threads",
        format!("{}", threads),
        "--preset",
        "medium",
        "--crf",
        format!("{}", q),
      ],
      Self::x265 => inplace_vec![
        "x265",
        "--log-level",
        "0",
        "--no-progress",
        "--y4m",
        "--frame-threads",
        format!("{}", cmp::min(threads, 16)),
        "--preset",
        "fast",
        "--crf",
        format!("{}", q),
      ],
    }
  }

  /// Returns command used for target quality probing (slow, correctness focused version)
  pub fn construct_target_quality_command_probe_slow(self, q: usize) -> Vec<Cow<'static, str>> {
    match &self {
      Self::aom => inplace_vec!["aomenc", "--passes=1", format!("--cq-level={}", q)],
      Self::rav1e => inplace_vec!["rav1e", "-y", "--quantizer", format!("{}", q)],
      Self::vpx => inplace_vec![
        "vpxenc",
        "--passes=1",
        "--pass=1",
        "--codec=vp9",
        "--end-usage=q",
        format!("--cq-level={}", q),
      ],
      Self::svt_av1 => inplace_vec!["SvtAv1EncApp", "-i", "stdin", "--crf", format!("{}", q)],
      Self::x264 => inplace_vec![
        "x264",
        "--log-level",
        "error",
        "--demuxer",
        "y4m",
        "-",
        "--no-progress",
        "--crf",
        format!("{}", q),
      ],
      Self::x265 => inplace_vec![
        "x265",
        "--log-level",
        "0",
        "--no-progress",
        "--y4m",
        "--crf",
        format!("{}", q),
      ],
    }
  }

  /// Function `remove_patterns` that takes in args and patterns and removes all instances of the patterns from the args.
  pub fn remove_patterns(args: &mut Vec<String>, patterns: &[&str]) {
    for pattern in patterns {
      if let Some(index) = args.iter().position(|value| value.contains(pattern)) {
        args.remove(index);
        // If pattern does not contain =, we need to remove the index that follows.
        if !pattern.contains('=') {
          args.remove(index);
        }
      }
    }
  }

  /// Constructs tuple of commands for target quality probing
  pub fn probe_cmd(
    self,
    temp: String,
    chunk_index: usize,
    q: usize,
    pix_fmt: Pixel,
    probing_rate: usize,
    vmaf_threads: usize,
    mut video_params: Vec<String>,
    probe_slow: bool,
  ) -> (Vec<String>, Vec<Cow<'static, str>>) {
    let pipe = compose_ffmpeg_pipe(
      [
        "-vf",
        format!("select=not(mod(n\\,{}))", probing_rate).as_str(),
        "-vsync",
        "0",
      ],
      pix_fmt,
    );

    let probe_name = format!("v_{}_{}.ivf", q, chunk_index);
    let mut probe = PathBuf::from(temp);
    probe.push("split");
    probe.push(&probe_name);
    let probe_path = probe.to_str().unwrap().to_owned();

    let params: Vec<Cow<str>> = if probe_slow {
      let patterns = [
        "--cq-level=",
        "--passes=",
        "--pass=",
        "--crf",
        "--quantizer",
      ];
      Self::remove_patterns(&mut video_params, &patterns);
      let mut ps = self.construct_target_quality_command_probe_slow(q);

      ps.reserve(video_params.len());
      for arg in video_params {
        ps.push(Cow::Owned(arg));
      }

      ps
    } else {
      self.construct_target_quality_command(vmaf_threads, q)
    };

    let output: Vec<Cow<str>> = match self {
      Self::svt_av1 => chain!(params, into_vec!["-b", probe_path]).collect(),
      Self::aom | Self::rav1e | Self::vpx | Self::x264 | Self::x265 => {
        chain!(params, into_vec!["-o", probe_path, "-"]).collect()
      }
    };

    (pipe, output)
  }

  pub fn get_format_bit_depth(self, format: Pixel) -> Result<usize, UnsupportedPixelFormatError> {
    macro_rules! impl_this_function {
      ($($encoder:ident),*) => {
        match self {
          $(
            Encoder::$encoder => paste::paste! { [<get_ $encoder _format_bit_depth>](format) },
          )*
        }
      };
    }
    impl_this_function!(x264, x265, vpx, aom, rav1e, svt_av1)
  }
}

#[derive(Error, Debug)]
pub enum UnsupportedPixelFormatError {
  #[error("{0} does not support {1:?}")]
  UnsupportedFormat(Encoder, Pixel),
}

macro_rules! create_get_format_bit_depth_function {
  ($encoder:ident, 8: $_8bit_fmts:expr, 10: $_10bit_fmts:expr, 12: $_12bit_fmts:expr) => {
    paste::paste! {
      pub fn [<get_ $encoder _format_bit_depth>](format: Pixel) -> Result<usize, UnsupportedPixelFormatError> {
        use Pixel::*;
        if $_8bit_fmts.contains(&format) {
          Ok(8)
        } else if $_10bit_fmts.contains(&format) {
          Ok(10)
        } else if $_12bit_fmts.contains(&format) {
          Ok(12)
        } else {
          Err(UnsupportedPixelFormatError::UnsupportedFormat(Encoder::$encoder, format))
        }
      }
    }
  };
}

// The supported bit depths are taken from ffmpeg,
// e.g.: `ffmpeg -h encoder=libx264`
create_get_format_bit_depth_function!(
  x264,
   8: [YUV420P, YUVJ420P, YUV422P, YUVJ422P, YUV444P, YUVJ444P, NV12, NV16, NV21, GRAY8],
  10: [YUV420P10LE, YUV422P10LE, YUV444P10LE, NV20LE, GRAY10LE],
  12: []
);
create_get_format_bit_depth_function!(
  x265,
   8: [YUV420P, YUVJ420P, YUV422P, YUVJ422P, YUV444P, YUVJ444P, GBRP, GRAY8],
  10: [YUV420P10LE, YUV422P10LE, YUV444P10LE, GBRP10LE, GRAY10LE],
  12: [YUV420P12LE, YUV422P12LE, YUV444P12LE, GBRP12LE, GRAY12LE]
);
create_get_format_bit_depth_function!(
  vpx,
   8: [YUV420P, YUVA420P, YUV422P, YUV440P, YUV444P, GBRP],
  10: [YUV420P10LE, YUV422P10LE, YUV440P10LE, YUV444P10LE, GBRP10LE],
  12: [YUV420P12LE, YUV422P12LE, YUV440P12LE, YUV444P12LE, GBRP12LE]
);
create_get_format_bit_depth_function!(
  aom,
   8: [YUV420P, YUV422P, YUV444P, GBRP, GRAY8],
  10: [YUV420P10LE, YUV422P10LE, YUV444P10LE, GBRP10LE, GRAY10LE],
  12: [YUV420P12LE, YUV422P12LE, YUV444P12LE, GBRP12LE, GRAY12LE,]
);
create_get_format_bit_depth_function!(
  rav1e,
   8: [YUV420P, YUVJ420P, YUV422P, YUVJ422P, YUV444P, YUVJ444P],
  10: [YUV420P10LE, YUV422P10LE, YUV444P10LE],
  12: [YUV420P12LE, YUV422P12LE, YUV444P12LE,]
);
create_get_format_bit_depth_function!(
  svt_av1,
   8: [YUV420P],
  10: [YUV420P10LE],
  12: []
);
