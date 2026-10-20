//! The live view of the conversation's browser: the user watches the tabs as
//! the Host renders them and operates them in the work panel while the agent
//! keeps using `demi browser` commands on the same tabs.
//!
//! This module is the Host's side: viewers, the capture of the tabs they watch,
//! its pacing, and the relay of their input to the tab. The mechanics shared
//! with the simulators' view are in `live-view` and `live-view-protocol`, the
//! browser's own messages in `command-package-browser-protocol`.
//!
//! ```text
//! User's browser           Backend                    Host (Cloud or device)
//! +----------------+       +-------------------+      +---------------------+
//! | plugin-browser |       | edge: checks      |      | runner              |
//! | video, input,  |  WSS  | cookie, owner,    | pipes|  -> demi-browser    |
//! | native         |<=====>| Origin; relays    |<====>|   conversation      |
//! | controls,      |       | the bytes         |      |   browser:          |
//! | viewport menu  |       |-------------------|      |    tab registry     |
//! +----------------+       | user's shard:     |      |    live hub         |
//!                          | admits the stream,|      |    capture channel  |
//!                          | opens its pipes   |      |   Chrome:           |
//!                          +-------------------+      |    capture ext.     |
//!                                                     |    pages, observers |
//!                                                     +---------------------+
//! ```
//!
//! # Rules
//!
//! - The runner and backend only carry bytes between this module and the page;
//!   neither knows a browser is involved. CDP stays on the Host and the page
//!   speaks only the live protocol.
//! - One view is one invocation of `browser.live`, for the conversation's
//!   primary Host only. It carries only what a request cannot: pictures, input
//!   and its answers, and what belongs to this viewer. Listing, opening,
//!   closing and navigating tabs are requests (the tab methods of
//!   `plugin-browser`) and work while no view is open.
//! - The module never fails silently toward the page: what it cannot do for a
//!   viewer it tells that viewer in a `notice` and writes to the Host's log.
//! - A hidden page holds no view: an open view keeps the conversation, and a
//!   Cloud, active, and nobody can watch a hidden page. A visible page keeps
//!   its view however long nobody touches it.
//! - Admission never wakes a stopped Cloud (it holds no browser). A shown tab
//!   that needs the Cloud wakes it by opening its tab, and the view connects
//!   again once it runs so the Host learns the panel's size before it creates
//!   the tab.
//! - Chrome's own screencast sends every frame as a separate image over CDP, on
//!   the connection that carries the agent's commands. Tab capture with H.264
//!   encodes only what changes and leaves CDP to the commands.
//! - The stream operates a browser signed in to the user's sites. The edge
//!   checks the session cookie, the conversation's owner and the product's
//!   origin before the request reaches the shard. The capture socket accepts
//!   only its environment's token. Page observers run in an isolated world.

use demi_command_package_browser_protocol::browser as protocol;

pub mod hub;
pub mod viewer;

mod commands;
mod downloads;
mod frames;
mod input;
mod observers;
mod stream;
mod uploads;
mod writer;
