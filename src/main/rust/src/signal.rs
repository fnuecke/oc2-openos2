// SPDX-License-Identifier: MIT

use crate::arch::KernelCell;
use crate::drivers::uart;
use crate::drivers::virtio::blk;
use crate::drivers::virtio::console as vcon;
use crate::drivers::virtio::snd;
use crate::fs::MountText;
use crate::fs::vfs::{self, Change};
use crate::power::{self, Request};
use crate::text::Text;
use crate::time::app_wait;

pub enum Event {
    Medium(MountText, Change),
    Port(vcon::Vport),
    Serial(Text<16>),
    Sound(Text<16>),
    Power(Request),
}

static STATE: KernelCell<State> = KernelCell::new(State { stage: Stage::Done, power_signaled: false });

pub fn wait(deadline: u64) {
    app_wait(pending, deadline);
}

pub fn collect() {
    vfs::poll_media();
    STATE.with(|s| s.stage = Stage::Medium);
}

pub fn next() -> Option<Event> {
    STATE.with(|s| {
        while s.stage != Stage::Done {
            if let Some(signal) = s.take() {
                return Some(signal);
            }
            s.stage = match s.stage {
                Stage::Medium => Stage::Port,
                Stage::Port => Stage::Serial,
                Stage::Serial => Stage::Sound,
                Stage::Sound => Stage::Power,
                Stage::Power | Stage::Done => Stage::Done,
            };
        }
        None
    })
}

fn pending() -> bool {
    vfs::medium_event_pending()
        || (0..blk::count()).filter_map(blk::get).any(blk::Disk::change_pending)
        || vcon::signal_ready()
        || uart::signal_ready()
        || snd::signal_ready()
        || (power::requested().is_some() && !STATE.with(|s| s.power_signaled))
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Done, // first, so that the state starts zeroed in the bss
    Medium,
    Port,
    Serial,
    Sound,
    Power,
}

struct State {
    stage: Stage,
    power_signaled: bool,
}

impl State {
    fn take(&mut self) -> Option<Event> {
        match self.stage {
            Stage::Medium => {
                let (path, change) = vfs::take_medium_event()?;
                Some(Event::Medium(path, change))
            }
            Stage::Port => Some(Event::Port(vcon::take_signal()?)),
            Stage::Serial => {
                let mut path = Text::of(b"/dev/");
                path.push(uart::name(uart::take_signal()?)?.as_bytes());
                Some(Event::Serial(path))
            }
            Stage::Sound => {
                let mut path = Text::of(b"/dev/");
                path.push(snd::take_signal()?.as_bytes());
                Some(Event::Sound(path))
            }
            Stage::Power => {
                let request = power::requested().filter(|_| !self.power_signaled)?;
                self.power_signaled = true;
                Some(Event::Power(request))
            }
            Stage::Done => None,
        }
    }
}
