use std::fmt::Debug;
use std::io::Cursor;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageBuffer, Rgba};
use image_to_binary_core::{ContractError, Dimensions, Generation, ResourceLimits, Sampling};
use image_to_binary_processing::{
    decode_animation, DecodeLimits, DecodedSource, LoopCount as ProcessingLoopCount,
};

use super::args::frame_index;
use super::plan::{animated_or_still, EncodedPlan};
use super::*;

const SIZE: u32 = 8;

fn gif(shades: &[u8], delays_centiseconds: &[u32], repeat: Option<Repeat>) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut encoder = GifEncoder::new(&mut output);
    if let Some(repeat) = repeat {
        encoder.set_repeat(repeat).unwrap();
    }
    let frames: Vec<Frame> = shades
        .iter()
        .zip(delays_centiseconds)
        .map(|(&shade, &centiseconds)| {
            Frame::from_parts(
                ImageBuffer::from_pixel(SIZE, SIZE, Rgba([shade, shade, shade, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(centiseconds * 10, 1),
            )
        })
        .collect();
    encoder.encode_frames(frames).unwrap();
    drop(encoder);
    output.into_inner()
}

fn animation(bytes: &[u8]) -> image_to_binary_processing::DecodedAnimation {
    decode_animation(bytes, DecodeLimits::default())
        .unwrap()
        .expect("multi-frame GIF")
}

fn three_frames() -> Session {
    let bytes = gif(&[255, 128, 0], &[10, 10, 10], None);
    Session::new_animation(&animation(&bytes)).unwrap()
}

fn g(value: u64) -> Generation {
    Generation::new(value)
}

fn settings(threshold: u8) -> PreviewSettings {
    PreviewSettings::new(5, 10, threshold, Sampling::Classic).unwrap()
}

fn code<T: Debug>(result: Result<T, SessionError>) -> ErrorCode {
    result.unwrap_err().code()
}

fn preview(session: &mut Session, generation: u64, frame: usize) -> Vec<u8> {
    session
        .start_preview_frame(g(generation), frame, settings(128))
        .unwrap();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(generation), u64::MAX).unwrap();
    }
    let mut rendered = session.finish(g(generation)).unwrap();
    rendered.take_pixels().unwrap()
}

fn export(session: &mut Session, generation: u64, frame: usize) -> Vec<u8> {
    session
        .start_export_frame_as(g(generation), frame, settings(128), OutputKind::Png)
        .unwrap();
    let mut bytes = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(generation), u64::MAX).unwrap();
        for chunk in session.drain_export(g(generation)).unwrap() {
            bytes.extend(chunk);
        }
    }
    for chunk in session.finish_export(g(generation)).unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

fn sampler_stats(session: &Session, frame: usize) -> (usize, u64, u64) {
    let live = session.live.as_ref().unwrap();
    let stats = live.frames[frame].sampler.cache_stats();
    (stats.entries(), stats.capacity(), stats.computed())
}

#[test]
fn animation_reports_frame_count_delays_and_loop_count() {
    let bytes = gif(&[255, 128, 0], &[5, 0, 50], Some(Repeat::Infinite));
    let session = Session::new_animation(&animation(&bytes)).unwrap();
    assert_eq!(session.frame_count().unwrap(), 3);
    assert_eq!(session.frame_delays_centiseconds().unwrap(), &[5, 10, 50]);
    assert_eq!(session.loop_count().unwrap(), LoopCount::Infinite);
}

#[test]
fn absent_loop_extension_plays_once_and_finite_counts_pass_through() {
    let absent = gif(&[255, 0], &[10, 10], None);
    let once = Session::new_animation(&animation(&absent)).unwrap();
    assert_eq!(once.loop_count().unwrap(), LoopCount::Finite(0));

    let finite = gif(&[255, 0], &[10, 10], Some(Repeat::Finite(3)));
    let session = Session::new_animation(&animation(&finite)).unwrap();
    assert_eq!(session.loop_count().unwrap(), LoopCount::Finite(3));
    assert_eq!(
        animation(&finite).loop_count(),
        ProcessingLoopCount::Finite(3)
    );
}

#[test]
fn still_source_has_one_frame_no_delays_and_plays_once() {
    let still = DecodedSource::from_rgba(
        Dimensions::new(SIZE, SIZE).unwrap(),
        vec![200; usize::try_from(SIZE * SIZE * 4).unwrap()],
        DecodeLimits::default(),
    )
    .unwrap();
    let mut session = Session::new(still).unwrap();
    assert_eq!(session.frame_count().unwrap(), 1);
    assert!(session.frame_delays_centiseconds().unwrap().is_empty());
    assert_eq!(session.loop_count().unwrap(), LoopCount::Finite(0));
    assert_eq!(
        code(session.start_preview_frame(g(0), 1, settings(128))),
        ErrorCode::InvalidArgument
    );
    assert_eq!(session.stage().unwrap(), Stage::Idle);
    session.start_preview(g(0), settings(128)).unwrap();
    assert_eq!(session.stage().unwrap(), Stage::Sampling);
}

#[test]
fn distinct_frames_render_distinct_previews_matching_their_stills() {
    let bytes = gif(&[255, 0], &[10, 10], None);
    let mut session = Session::new_animation(&animation(&bytes)).unwrap();
    let white = preview(&mut session, 0, 0);
    let black = preview(&mut session, 1, 1);
    assert_ne!(white, black);

    let decoded = animation(&bytes);
    let frame_pixels = decoded.frames()[1].source().pixels().to_vec();
    let still = DecodedSource::from_rgba(
        Dimensions::new(SIZE, SIZE).unwrap(),
        frame_pixels,
        DecodeLimits::default(),
    )
    .unwrap();
    let mut reference = Session::new(still).unwrap();
    assert_eq!(preview(&mut reference, 0, 0), black);
}

#[test]
fn frame_exports_differ_and_follow_the_frame_index() {
    let bytes = gif(&[255, 0], &[10, 10], None);
    let mut session = Session::new_animation(&animation(&bytes)).unwrap();
    let first = export(&mut session, 0, 0);
    let second = export(&mut session, 1, 1);
    assert_ne!(first, second);
    assert!(first.starts_with(b"\x89PNG"));
}

#[test]
fn out_of_range_frames_are_typed_and_keep_the_current_job() {
    let mut session = three_frames();
    session.start_preview_frame(g(0), 0, settings(128)).unwrap();
    let error = session
        .start_preview_frame(g(1), 3, settings(128))
        .unwrap_err();
    assert_eq!(error, SessionError::FrameOutOfRange { frame: 3, count: 3 });
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert_eq!(
        code(session.start_export_frame_as(g(1), 9, settings(128), OutputKind::Png)),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        code(session.start_original_preview_frame(g(1), 3)),
        ErrorCode::InvalidArgument
    );
    assert_eq!(session.stage().unwrap(), Stage::Sampling);
    session.start_original_preview_frame(g(2), 2).unwrap();
}

#[test]
fn a_stale_generation_is_refused_across_frames() {
    let mut session = three_frames();
    session.start_preview_frame(g(5), 2, settings(128)).unwrap();
    assert_eq!(
        code(session.start_preview_frame(g(5), 0, settings(128))),
        ErrorCode::StaleGeneration
    );
}

#[test]
fn animated_frames_hold_no_sample_cache_while_stills_keep_theirs() {
    let mut session = three_frames();
    let _ = preview(&mut session, 0, 1);
    let _ = preview(&mut session, 1, 1);
    let (entries, capacity, computed) = sampler_stats(&session, 1);
    assert_eq!((entries, capacity), (0, 0));
    assert_eq!(computed, 2, "each preview samples its frame again");

    let still = DecodedSource::from_rgba(
        Dimensions::new(SIZE, SIZE).unwrap(),
        vec![10; usize::try_from(SIZE * SIZE * 4).unwrap()],
        DecodeLimits::default(),
    )
    .unwrap();
    let mut single = Session::new(still).unwrap();
    let _ = preview(&mut single, 0, 0);
    let (entries, capacity, _) = sampler_stats(&single, 0);
    assert_eq!(entries, 1);
    assert_eq!(capacity, 64 * 1024 * 1024);
}

#[test]
fn frame_index_accepts_safe_integers_only() {
    assert_eq!(frame_index(0.0).unwrap(), 0);
    assert_eq!(frame_index(2.0).unwrap(), 2);
    for invalid in [-1.0, 1.5, f64::NAN, f64::INFINITY, 9_007_199_254_740_992.0] {
        assert_eq!(
            frame_index(invalid).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }
}

#[test]
fn a_thousand_frames_import_as_an_animation_and_one_thousand_one_falls_back_to_the_first() {
    let accepted = vec![10_u8; 1000];
    let bytes = gif(&accepted, &vec![1; 1000], None);
    let session = EncodedPlan::decode_animated_session(bytes).unwrap();
    assert_eq!(session.frame_count().unwrap(), 1000);

    let rejected = vec![10_u8; 1001];
    let bytes = gif(&rejected, &vec![1; 1001], None);
    let session = EncodedPlan::decode_animated_session(bytes).unwrap();
    assert_eq!(session.frame_count().unwrap(), 1);
    assert!(session.frame_delays_centiseconds().unwrap().is_empty());
}

#[test]
fn the_decoded_byte_budget_falls_back_to_the_first_frame() {
    let bytes = gif(&[255, 128, 0], &[10, 10, 10], None);
    let frame_bytes = u64::from(SIZE) * u64::from(SIZE) * 4;
    let tight = DecodeLimits::new(
        ResourceLimits::default(),
        256 * 1024 * 1024,
        frame_bytes * 3,
    )
    .unwrap();
    let session = animated_or_still(&bytes, tight).unwrap();
    assert_eq!(session.frame_count().unwrap(), 1);

    let roomy = DecodeLimits::new(
        ResourceLimits::default(),
        256 * 1024 * 1024,
        frame_bytes * 4,
    )
    .unwrap();
    let session = animated_or_still(&bytes, roomy).unwrap();
    assert_eq!(session.frame_count().unwrap(), 3);
}

#[test]
fn malformed_animations_report_their_decode_error_instead_of_falling_back() {
    let mut bytes = gif(&[255, 0], &[10, 10], None);
    bytes.truncate(bytes.len() - 8);
    let error = match EncodedPlan::decode_animated_session(bytes) {
        Ok(_) => panic!("truncated animation must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code(), ErrorCode::DecodeFailed);
    assert_ne!(
        error,
        SessionError::Contract(ContractError::ResourceLimitExceeded)
    );
}

fn still() -> Session {
    let source = DecodedSource::from_rgba(
        Dimensions::new(8, 8).unwrap(),
        vec![255; 8 * 8 * 4],
        DecodeLimits::default(),
    )
    .unwrap();
    Session::new(source).unwrap()
}

fn gif_export(session: &mut Session, generation: u64, budget: u64) -> Vec<u8> {
    session
        .start_export_frame_as(g(generation), 0, settings(128), OutputKind::Gif)
        .unwrap();
    let mut bytes = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(generation), budget).unwrap();
        for chunk in session.drain_export(g(generation)).unwrap() {
            bytes.extend(chunk);
        }
    }
    for chunk in session.finish_export(g(generation)).unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

fn gif_delays_centiseconds(bytes: &[u8]) -> Vec<u64> {
    let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).unwrap();
    let frames = image::AnimationDecoder::into_frames(decoder)
        .collect_frames()
        .unwrap();
    frames
        .iter()
        .map(|frame| {
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            u64::from(numerator) / u64::from(denominator) / 10
        })
        .collect()
}

#[test]
fn a_gif_export_writes_every_frame_with_its_timing_and_loop() {
    let bytes = gif(&[255, 128, 0], &[5, 10, 50], Some(Repeat::Infinite));
    let mut session = Session::new_animation(&animation(&bytes)).unwrap();
    let exported = gif_export(&mut session, 1, 37);
    assert!(exported.starts_with(b"GIF89a"));
    assert_eq!(gif_delays_centiseconds(&exported), [5, 10, 50]);
    assert!(exported.windows(11).any(|window| window == b"NETSCAPE2.0"));
}

#[test]
fn gif_bytes_do_not_depend_on_the_pixel_budget_and_play_once_without_a_loop() {
    let bytes = gif(&[255, 128, 0], &[10, 10, 10], None);
    let expected = gif_export(
        &mut Session::new_animation(&animation(&bytes)).unwrap(),
        1,
        u64::MAX,
    );
    assert!(!expected.windows(11).any(|window| window == b"NETSCAPE2.0"));
    for budget in [1, 37, 4096] {
        let mut session = Session::new_animation(&animation(&bytes)).unwrap();
        assert_eq!(gif_export(&mut session, 1, budget), expected);
    }
}

#[test]
fn a_gif_export_of_a_still_source_is_refused_without_a_job() {
    let mut session = still();
    let error = session
        .start_export_frame_as(g(1), 0, settings(128), OutputKind::Gif)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert_eq!(session.stage().unwrap(), Stage::Idle);
}

#[test]
fn a_gif_export_refuses_a_frame_index_and_reports_progress_in_frames() {
    let mut session = three_frames();
    let error = session
        .start_export_frame_as(g(1), 1, settings(128), OutputKind::Gif)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidArgument);

    session
        .start_export_frame_as(g(1), 0, settings(128), OutputKind::Gif)
        .unwrap();
    assert_eq!(session.stage().unwrap(), Stage::Encoding);
    assert_eq!(session.progress().unwrap(), (0, 3));
    let mut completed = Vec::new();
    while session.stage().unwrap() != Stage::Ready {
        session.advance(g(1), 4096).unwrap();
        completed.push(session.progress().unwrap().0);
    }
    assert!(completed.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(completed.last(), Some(&3));
    assert_eq!(session.advance(g(1), 4096).unwrap(), Stage::Ready);
    assert!(session.finish_export(g(1)).is_ok());
}

#[test]
fn cancelling_a_gif_export_mid_run_is_terminal() {
    let mut session = three_frames();
    session
        .start_export_frame_as(g(1), 0, settings(128), OutputKind::Gif)
        .unwrap();
    session.advance(g(1), 1).unwrap();
    session.cancel(g(1)).unwrap();
    assert_eq!(code(session.advance(g(1), 1)), ErrorCode::Cancelled);
    assert_eq!(code(session.drain_export(g(1))), ErrorCode::Cancelled);
    assert_eq!(code(session.finish_export(g(1))), ErrorCode::Cancelled);
}
