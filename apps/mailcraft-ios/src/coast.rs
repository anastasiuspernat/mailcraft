// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The trackpad's scrolling as egui's own events, with the coasting the system gives a scroll view
//! and not a gesture recognizer: when the fingers lift, the content goes on at the speed they lifted
//! at and slows as a UIKit scroll view does (it keeps 0.998 of its speed a millisecond), until it
//! is slow enough to stop, the fingers scroll again or something is pressed.
//!
//! The fingers' part is egui's `Trackpad` source and the coasting its `Momentum` source, as on a
//! Mac: at an edge egui stretches the content while fingers pull and bounces it once when coasting
//! hits it (coasting as fingers would hold the content stretched until the coasting is over).

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, Vec2};

/// What the trackpad did, in egui's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// The pointer is here.
    Moved(Pos2),
    /// It left the window.
    Gone,
    /// The fingers began scrolling with the pointer here.
    Start(Pos2),
    /// The content moved by this much (natural scrolling) with the pointer here.
    Scroll(Pos2, Vec2),
    /// The fingers lifted, moving at this many points a second.
    End(Vec2),
}

/// How long the speed takes to fall to 1/e: a UIKit scroll view's normal deceleration, 0.998 of the
/// speed kept each millisecond (-1 ms / ln 0.998).
const DECAY_TIME: f32 = 0.4995;
/// Below this many points a second, coasting stops (and lifted fingers don't start it).
const SLOWEST: f32 = 15.0;
/// The most a coast starts at, in points a second.
const FASTEST: f32 = 8000.0;
/// The longest step one frame coasts, in seconds (a frame that came late doesn't jump).
const LONGEST_STEP: f64 = 0.1;

#[derive(Debug, Default)]
pub struct Trackpad {
    /// A scroll's end and what came after it, for the next frame: egui drops what a frame
    /// scrolled when the same frame ends the scroll.
    held: Vec<Input>,
    coast: Option<Coast>,
}

#[derive(Clone, Copy, Debug)]
struct Coast {
    /// Points a second.
    speed: Vec2,
    /// When it last moved, in seconds.
    last: f64,
    /// egui has been told it started.
    started: bool,
    /// It has stopped: egui is told next frame.
    ending: bool,
}

fn wheel(delta: Vec2, phase: TouchPhase, source: MouseWheelSource, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase, source, modifiers }
}

impl Trackpad {
    /// One frame at `now` (seconds): what the trackpad did since the last (`seen`), and whether
    /// a button or a finger was pressed in it, as events to `out`. True when another frame is
    /// wanted (coasting, or something held for it).
    pub fn frame(&mut self, now: f64, seen: Vec<Input>, pressed: bool, modifiers: Modifiers, out: &mut Vec<Event>) -> bool {
        let mut all = std::mem::take(&mut self.held);
        all.extend(seen);
        let ends_after_a_move = all.iter().enumerate().find_map(|(i, p)| {
            (matches!(p, Input::End(_)) && all.get(..i).is_some_and(|before| before.iter().any(|q| matches!(q, Input::Scroll(..))))).then_some(i)
        });
        if let Some(i) = ends_after_a_move {
            self.held = all.split_off(i);
        }
        if pressed {
            self.stop(modifiers, out);
        }
        for input in all {
            match input {
                Input::Moved(at) => out.push(Event::PointerMoved(at)),
                Input::Gone => out.push(Event::PointerGone),
                Input::Start(at) => {
                    self.stop(modifiers, out);
                    out.push(Event::PointerMoved(at));
                    out.push(wheel(Vec2::ZERO, TouchPhase::Start, MouseWheelSource::Trackpad, modifiers));
                }
                Input::Scroll(at, delta) => {
                    out.push(Event::PointerMoved(at));
                    out.push(wheel(delta, TouchPhase::Move, MouseWheelSource::Trackpad, modifiers));
                }
                Input::End(speed) => {
                    out.push(wheel(Vec2::ZERO, TouchPhase::End, MouseWheelSource::Trackpad, modifiers));
                    let speed = if speed.is_finite() { speed * (FASTEST / speed.length().max(FASTEST)) } else { Vec2::ZERO };
                    self.coast = (SLOWEST <= speed.length()).then_some(Coast { speed, last: now, started: false, ending: false });
                }
            }
        }
        let coasting = self.coast_on(now, modifiers, out);
        coasting || !self.held.is_empty()
    }

    /// The coast's step for this frame; true while it goes on.
    fn coast_on(&mut self, now: f64, modifiers: Modifiers, out: &mut Vec<Event>) -> bool {
        let Some(coast) = &mut self.coast else { return false };
        if coast.ending {
            out.push(wheel(Vec2::ZERO, TouchPhase::End, MouseWheelSource::Momentum, modifiers));
            self.coast = None;
            return false;
        }
        if !coast.started {
            out.push(wheel(Vec2::ZERO, TouchPhase::Start, MouseWheelSource::Momentum, modifiers));
            coast.started = true;
            coast.last = now;
            return true;
        }
        let dt = (now - coast.last).clamp(0.0, LONGEST_STEP) as f32;
        coast.last = now;
        let slower = coast.speed * (-dt / DECAY_TIME).exp();
        // The exact distance over the step, so that how far it coasts doesn't depend on the frame rate.
        let delta = (coast.speed - slower) * DECAY_TIME;
        coast.speed = slower;
        if delta != Vec2::ZERO {
            out.push(wheel(delta, TouchPhase::Move, MouseWheelSource::Momentum, modifiers));
        }
        if slower.length() < SLOWEST {
            coast.ending = true;
        }
        true
    }

    /// Stop coasting, now.
    fn stop(&mut self, modifiers: Modifiers, out: &mut Vec<Event>) {
        if let Some(coast) = self.coast.take()
            && coast.started
        {
            out.push(wheel(Vec2::ZERO, TouchPhase::End, MouseWheelSource::Momentum, modifiers));
        }
    }
}
