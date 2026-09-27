//! Video and shader fills that play: in the editor, the Preview of the
//! selected shapes; in a presentation, the fills of the slide on screen.
//!
//! Each playing fill has a player (a video) or a clock (a shader). The
//! views ask for the picture of a fill on each animation frame with
//! [`Playback::picture`]; a fill that is not playing shows its still
//! picture.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tiny_skia::Pixmap;

use crate::document::{ElementId, Fill, Presentation};
use crate::shaders::Inputs;
use crate::videos::Player;

/// Seconds that run while playing and stop while paused.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    /// When it last started to run; `None` while paused.
    started: Option<Instant>,
    /// Seconds run before it last started.
    before: f32,
}

impl Clock {
    pub fn play(&mut self) {
        if self.started.is_none() {
            self.started = Some(Instant::now());
        }
    }

    pub fn pause(&mut self) {
        if let Some(started) = self.started.take() {
            self.before += started.elapsed().as_secs_f32();
        }
    }

    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    /// Seconds run in total.
    pub fn elapsed(&self) -> f32 {
        self.before
            + self
                .started
                .map_or(0., |started| started.elapsed().as_secs_f32())
    }
}

enum Live {
    Video(Player),
    Shader {
        clock: Clock,
        /// The last rendered frame and the time and size it shows.
        last: Option<(f32, (u32, u32), Arc<Pixmap>)>,
    },
}

/// The fills that play, by element.
#[derive(Default)]
pub struct Playback {
    live: HashMap<ElementId, Live>,
    /// Plays videos without sound, as the Preview of the editor does.
    pub mute: bool,
}

/// Most pixels on a side of a live video frame.
const VIDEO_SIDE: u32 = 1920;

impl Playback {
    /// Playback that plays videos without sound.
    pub fn muted() -> Self {
        Playback {
            mute: true,
            ..Playback::default()
        }
    }

    /// Starts the video or shader fill of an element, from its start; a
    /// fill that already plays goes on. Returns whether the fill plays.
    pub fn start(&mut self, id: ElementId, fill: &Fill, presentation: &Presentation) -> bool {
        if let Some(live) = self.live.get_mut(&id) {
            match live {
                Live::Video(player) => player.play(),
                Live::Shader { clock, .. } => clock.play(),
            }
            return true;
        }
        let live = match fill {
            Fill::Video(video) => {
                let Some(data) = presentation.videos.get(video.id) else {
                    return false;
                };
                // Tests play without sound, so that they need no audio
                // output.
                let muted = video.muted || self.mute || cfg!(test);
                let Ok(mut player) = Player::new(data, VIDEO_SIDE, video.looped, muted) else {
                    return false;
                };
                player.play();
                Live::Video(player)
            }
            Fill::Shader(_) => {
                let mut clock = Clock::default();
                clock.play();
                Live::Shader { clock, last: None }
            }
            _ => return false,
        };
        self.live.insert(id, live);
        true
    }

    /// Pauses a playing fill, or plays a paused one.
    pub fn toggle(&mut self, id: ElementId) {
        match self.live.get_mut(&id) {
            Some(Live::Video(player)) if player.playing() => player.pause(),
            Some(Live::Video(player)) => player.play(),
            Some(Live::Shader { clock, .. }) if clock.running() => clock.pause(),
            Some(Live::Shader { clock, .. }) => clock.play(),
            None => {}
        }
    }

    /// Stops a fill: it shows its still picture again.
    pub fn stop(&mut self, id: ElementId) {
        self.live.remove(&id);
    }

    pub fn clear(&mut self) {
        self.live.clear();
    }

    /// Whether the fill of the element started, playing or paused.
    pub fn is_live(&self, id: ElementId) -> bool {
        self.live.contains_key(&id)
    }

    /// The elements whose fill started.
    pub fn ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.live.keys().copied()
    }

    /// Whether the fill of the element plays now.
    pub fn is_playing(&self, id: ElementId) -> bool {
        match self.live.get(&id) {
            Some(Live::Video(player)) => player.playing(),
            Some(Live::Shader { clock, .. }) => clock.running(),
            None => false,
        }
    }

    /// Whether a fill plays, so that the view needs a new frame soon.
    pub fn animating(&self) -> bool {
        self.live.values().any(|live| match live {
            // A video that ended still looks for its end, once.
            Live::Video(player) => player.playing(),
            Live::Shader { clock, .. } => clock.running(),
        })
    }

    /// The picture of a started fill now; `None` for a fill that did not
    /// start or has no picture yet. `size` is the size in pixels a shader
    /// renders at.
    pub fn picture(
        &mut self,
        id: ElementId,
        fill: &Fill,
        presentation: &Presentation,
        size: (u32, u32),
    ) -> Option<Arc<Pixmap>> {
        match (self.live.get_mut(&id)?, fill) {
            (Live::Video(player), Fill::Video(_)) => player.frame(),
            (Live::Shader { clock, last }, Fill::Shader(shader)) => {
                let time = shader.time(clock.elapsed());
                if let Some((shown, shown_size, pixels)) = last
                    && *shown == time
                    && *shown_size == size
                {
                    return Some(pixels.clone());
                }
                let channel = match shader.channel0 {
                    Some(image) => Some(crate::images::pixels(presentation.images.get(image)?)?),
                    None => None,
                };
                let size = crate::shaders::clamp_size(size.0, size.1);
                let pixels = Arc::new(
                    crate::shaders::render(
                        &shader.source,
                        channel.as_ref(),
                        size.0,
                        size.1,
                        Inputs::at(time),
                    )
                    .ok()?,
                );
                if !shader.looped && time >= shader.duration {
                    // A shader that does not loop stops at its duration.
                    clock.pause();
                }
                *last = Some((time, size, pixels.clone()));
                Some(pixels)
            }
            // The fill changed kind since it started.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ShaderFill;

    #[test]
    fn a_clock_counts_only_while_it_runs() {
        let mut clock = Clock::default();
        assert_eq!(clock.elapsed(), 0.);
        clock.play();
        std::thread::sleep(std::time::Duration::from_millis(20));
        clock.pause();
        let paused = clock.elapsed();
        assert!(paused >= 0.02, "{paused}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(clock.elapsed(), paused);
    }

    #[test]
    fn shaders_play_pause_and_stop() {
        let presentation = Presentation::new();
        let fill = Fill::Shader(ShaderFill::default());
        let mut playback = Playback::default();
        let id = ElementId(1);
        assert!(playback.picture(id, &fill, &presentation, (8, 8)).is_none());
        assert!(playback.start(id, &fill, &presentation));
        assert!(playback.is_playing(id) && playback.animating());
        playback.toggle(id);
        assert!(!playback.is_playing(id) && playback.is_live(id));
        if crate::shaders::available() {
            let first = playback.picture(id, &fill, &presentation, (8, 8)).unwrap();
            let again = playback.picture(id, &fill, &presentation, (8, 8)).unwrap();
            assert!(Arc::ptr_eq(&first, &again), "paused: the same frame");
        }
        playback.stop(id);
        assert!(!playback.is_live(id));
        assert!(!playback.start(id, &Fill::None, &presentation));
    }

    #[test]
    fn videos_give_new_frames_while_they_play() {
        if !crate::videos::tests::plugins_or_skip() {
            return;
        }
        let mut presentation = Presentation::new();
        let data = crate::videos::VideoData::read(crate::videos::tests::mp4(320, 180)).unwrap();
        let video = presentation.new_video_id();
        presentation
            .apply(crate::document::Operation::AddVideo { id: video, data })
            .unwrap();
        let fill = Fill::Video(crate::document::VideoFill::new(video));
        let mut playback = Playback::muted();
        let id = ElementId(1);
        assert!(playback.start(id, &fill, &presentation));
        assert!(playback.animating(), "it plays as soon as it starts");
        let mut frames = Vec::new();
        let start = Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(800) {
            if let Some(frame) = playback.picture(id, &fill, &presentation, (64, 64))
                && frames
                    .last()
                    .is_none_or(|last: &Arc<Pixmap>| !Arc::ptr_eq(last, &frame))
            {
                frames.push(frame);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(frames.len() > 3, "{} frames", frames.len());
    }
}
