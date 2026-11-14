// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use egui::{Event, Modifiers, MouseWheelSource, Pos2, TouchPhase, Vec2, pos2, vec2};

use crate::coast::{Input, Trackpad};

const AT: Pos2 = pos2(100.0, 200.0);

fn run(t: &mut Trackpad, now: f64, seen: Vec<Input>, pressed: bool) -> (Vec<Event>, bool) {
    let mut out = Vec::new();
    let again = t.frame(now, seen, pressed, Modifiers::NONE, &mut out);
    (out, again)
}

/// (delta, phase, source) of each wheel event.
fn wheels(events: &[Event]) -> Vec<(Vec2, TouchPhase, MouseWheelSource)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::MouseWheel { delta, phase, source, .. } => Some((*delta, *phase, *source)),
            _ => None,
        })
        .collect()
}

/// Fingers that scroll and lift at `speed`, over two frames (the end waits for the second).
fn scroll_and_lift(t: &mut Trackpad, speed: Vec2) -> Vec<Event> {
    let (first, again) = run(t, 0.0, vec![Input::Start(AT), Input::Scroll(AT, vec2(0.0, -10.0)), Input::End(speed)], false);
    assert!(again, "the end is held for the next frame");
    assert!(wheels(&first).iter().all(|w| w.1 != TouchPhase::End), "no end in the frame that scrolled");
    let (second, _) = run(t, 1.0 / 60.0, vec![], false);
    [first, second].concat()
}

#[test]
fn the_fingers_scroll_as_a_trackpad() {
    let mut t = Trackpad::default();
    let events = scroll_and_lift(&mut t, Vec2::ZERO);
    let w = wheels(&events);
    assert_eq!(w.first(), Some(&(Vec2::ZERO, TouchPhase::Start, MouseWheelSource::Trackpad)));
    assert!(w.contains(&(vec2(0.0, -10.0), TouchPhase::Move, MouseWheelSource::Trackpad)));
    assert_eq!(w.last(), Some(&(Vec2::ZERO, TouchPhase::End, MouseWheelSource::Trackpad)));
    assert!(!w.iter().any(|x| x.2 == MouseWheelSource::Momentum), "lifted still: no coasting");
}

#[test]
fn it_coasts_slower_and_slower_then_stops() {
    let mut t = Trackpad::default();
    let events = scroll_and_lift(&mut t, vec2(0.0, -2000.0));
    assert!(wheels(&events).contains(&(Vec2::ZERO, TouchPhase::Start, MouseWheelSource::Momentum)));
    let mut steps = Vec::new();
    let mut now = 1.0 / 60.0;
    let mut ended = false;
    for _ in 0..1000 {
        now += 1.0 / 60.0;
        let (out, again) = run(&mut t, now, vec![], false);
        for (delta, phase, source) in wheels(&out) {
            assert_eq!(source, MouseWheelSource::Momentum);
            match phase {
                TouchPhase::Move => steps.push(delta.y),
                TouchPhase::End => ended = true,
                _ => {}
            }
        }
        if !again {
            break;
        }
    }
    assert!(ended, "the coast ends");
    assert!(steps.windows(2).all(|w| w[0] <= w[1] && w[1] < 0.0), "upwards, slower each step: {steps:?}");
    // Speed × decay time, less the bit below the slowest speed: about a thousand points.
    let total: f32 = steps.iter().sum();
    assert!((-1000.0..-950.0).contains(&total), "{total}");
}

#[test]
fn scrolling_again_or_pressing_stops_the_coast() {
    for again_by_scrolling in [true, false] {
        let mut t = Trackpad::default();
        scroll_and_lift(&mut t, vec2(0.0, 3000.0));
        let (out, _) = run(&mut t, 0.05, vec![], false);
        assert!(wheels(&out).iter().any(|w| w.1 == TouchPhase::Move), "coasting");
        let seen = if again_by_scrolling { vec![Input::Start(AT)] } else { vec![] };
        let (out, _) = run(&mut t, 0.07, seen, !again_by_scrolling);
        let w = wheels(&out);
        assert_eq!(w.first(), Some(&(Vec2::ZERO, TouchPhase::End, MouseWheelSource::Momentum)));
        assert!(!w.iter().any(|x| x.1 == TouchPhase::Move && x.2 == MouseWheelSource::Momentum));
        let (out, again) = run(&mut t, 0.09, vec![], false);
        assert!(wheels(&out).is_empty() && !again, "nothing more");
    }
}

#[test]
fn a_huge_or_broken_speed_is_tamed() {
    for speed in [vec2(0.0, 1e9), vec2(f32::NAN, 5.0), vec2(f32::INFINITY, 0.0)] {
        let mut t = Trackpad::default();
        scroll_and_lift(&mut t, speed);
        for i in 0..2000 {
            let (out, again) = run(&mut t, 0.02 + f64::from(i) / 60.0, vec![], false);
            for (delta, ..) in wheels(&out) {
                assert!(delta.is_finite() && delta.length() <= 8000.0 / 60.0 + 1.0, "{delta:?}");
            }
            if !again {
                break;
            }
        }
    }
}
