// SPDX-License-Identifier: MIT

pub mod console;
pub mod plic;
pub mod rtc;
pub mod syscon;
pub mod uart;
pub mod virtio;

use crate::board::board;
use crate::fdt::Node;

pub struct Driver {
    /// Matches a node with any of these in its compatible list.
    pub compatible: &'static [&'static [u8]],
    /// Called for each matching node, in the order of the tree.
    pub probe: fn(&Node),
}

pub(crate) const VIRTIO_DRIVERS: &[&virtio::Driver] =
    &[&virtio::blk::DRIVER, &virtio::console::DRIVER, &crate::fs::p9::DRIVER];

const DRIVERS: &[&Driver] =
    &[&uart::DRIVER, &rtc::DRIVER, &syscon::DRIVER, &syscon::POWEROFF, &syscon::REBOOT, &virtio::DRIVER];

pub fn probe() {
    board().fdt.walk(&mut |node| {
        if node.prop(b"status").is_some_and(|status| !matches!(status, b"okay\0" | b"ok\0")) {
            return;
        }
        if let Some(driver) = DRIVERS.iter().find(|d| node.is_compatible(d.compatible)) {
            (driver.probe)(node);
        }
    });
}
