use shared::fmt_lite::StringOnStack;
use shared::io::{write_stderr, write_stdout};
use wllib::dispatch::EventHandler;
use wllib::error::WireError;
use wllib::protocols::{wl_output, zxdg_output_manager_v1, zxdg_output_v1};
use wllib::registry::{GlobalHandler, bind, clamp_version};
use wllib::transport::{Connection, Ctx};
use wllib::wire::{Message, Writer};

const MAX_OUTPUTS: usize = 8;

// `wl_output::geometry`'s `transform` arg, decoded from `wayland.xml`'s `wl_output.transform`
// enum (0-7).
fn transform_name(t: u32) -> &'static str {
  match t {
    0 => "normal",
    1 => "90",
    2 => "180",
    3 => "270",
    4 => "flipped",
    5 => "flipped-90",
    6 => "flipped-180",
    7 => "flipped-270",
    _ => "unknown",
  }
}

// `wl_output::geometry`'s `subpixel` arg, decoded from `wayland.xml`'s `wl_output.subpixel`
// enum (0-5).
fn subpixel_name(s: i32) -> &'static str {
  match s {
    0 => "unknown",
    1 => "none",
    2 => "horizontal-rgb",
    3 => "horizontal-bgr",
    4 => "vertical-rgb",
    5 => "vertical-bgr",
    _ => "unknown",
  }
}

// tracks registered object ids
#[derive(Default)]
pub struct Global {
  // wl_compositor global id
  pub compositor_id: u32,
  // zxdg_output_manager_v1 global id, 0 if the compositor doesn't advertise it. when absent,
  // outputs are printed with physical-only geometry from `wl_output`, no logical position/size.
  pub xdg_output_manager_id: u32,
}

#[derive(Default)]
pub struct Mode {
  pub flags: u32,
  pub width: i32,
  pub height: i32,
  pub refresh: i32,
}

#[derive(Default)]
pub struct Geometry {
  pub x: i32,
  pub y: i32,
  pub physical_width: i32,
  pub physical_height: i32,
  pub subpixel: i32,
  pub make: StringOnStack<64>,
  pub model: StringOnStack<64>,
  pub transform: u32,
}

#[derive(Default)]
pub struct Logical {
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
}

// abstration around wl_output
#[derive(Default)]
pub struct Output {
  pub output_id: u32,
  pub name: StringOnStack<64>,
  pub desc: StringOnStack<64>,
  pub geo: Geometry,
  pub mode: Mode,
  pub scale: i32,
  // id of this output's zxdg_output_v1 object, or 0 if the compositor has no
  // zxdg_output_manager_v1 global or `request_xdg_outputs` hasn't run yet.
  pub xdg_output_id: u32,
  pub logical: Logical,
  // set once wl_output::done has fired for this output.
  wl_done: bool,
  // set once zxdg_output_v1::logical_size has fired for this output. `is_complete` ignores it when
  // `xdg_output_id` is 0.
  logical_done: bool,
  // guard against printing/counting the same output twice.
  printed: bool,
}
impl Output {
  // an output is ready to print once wl_output has finished reporting, and once zxdg_output_v1 has
  // finished reporting, if one was actually requested for it.
  fn is_complete(&self) -> bool {
    self.wl_done && (self.xdg_output_id == 0 || self.logical_done)
  }
}

pub struct State {
  pub global: Global,
  pub outputs: [Output; MAX_OUTPUTS],
  pub output_len: usize,
  // count of outputs that have printed.
  pub done_count: usize,
}

impl State {
  pub fn init() -> Self {
    Self {
      global: Global::default(),
      outputs: core::array::from_fn(|_| Output::default()),
      output_len: 0,
      done_count: 0,
    }
  }

  // send `get_xdg_output(new_id, wl_output_id)` for every bound output, once the manager global
  // (if any) is known. called once after `crawl()` returns.
  pub fn request_xdg_outputs(&mut self, conn: &mut Connection) -> Result<(), WireError> {
    let manager_id = self.global.xdg_output_manager_id;
    if manager_id == 0 {
      return Ok(());
    }
    for out in &mut self.outputs[..self.output_len] {
      let id = conn.alloc_id()?;
      let mut w = Writer::<16>::new(manager_id, zxdg_output_manager_v1::request::GET_XDG_OUTPUT);
      w.u32(id).u32(out.output_id);
      conn.send(w.finish()?, &[])?;
      out.xdg_output_id = id;
    }
    Ok(())
  }

  // returns `true` once this call has made the output ready to print (see `Output::is_complete`),
  // so the caller can print it and count it toward `done_count` exactly once.
  fn handle_output_event(out: &mut Output, msg: &Message<'_>) -> Result<bool, WireError> {
    let mut r = msg.args();
    match msg.header.opcode {
      wl_output::event::NAME => {
        out.name.push(r.str()?);
      }
      wl_output::event::DESCRIPTION => {
        out.desc.push(r.str()?);
      }
      wl_output::event::GEOMETRY => {
        out.geo.x = r.i32()?;
        out.geo.y = r.i32()?;
        out.geo.physical_width = r.i32()?;
        out.geo.physical_height = r.i32()?;
        out.geo.subpixel = r.i32()?;
        out.geo.make.push(r.str()?);
        out.geo.model.push(r.str()?);
        out.geo.transform = r.u32()?;
      }
      wl_output::event::MODE => {
        out.mode.flags = r.u32()?;
        out.mode.width = r.i32()?;
        out.mode.height = r.i32()?;
        out.mode.refresh = r.i32()?;
      }
      wl_output::event::SCALE => out.scale = r.i32()?,
      wl_output::event::DONE => out.wl_done = true,
      _ => (),
    }
    Ok(Self::finish_if_complete(out))
  }

  fn handle_xdg_output_event(out: &mut Output, msg: &Message<'_>) -> Result<bool, WireError> {
    let mut r = msg.args();
    match msg.header.opcode {
      zxdg_output_v1::event::LOGICAL_POSITION => {
        out.logical.x = r.i32()?;
        out.logical.y = r.i32()?;
      }
      zxdg_output_v1::event::LOGICAL_SIZE => {
        out.logical.width = r.i32()?;
        out.logical.height = r.i32()?;
        // only ever sent once per property change and always accompanies a full update, so it's a
        // safe, version-independent "logical info is current" signal without needing to also handle
        // the deprecated `done` event.
        out.logical_done = true;
      }
      _ => (),
    }
    Ok(Self::finish_if_complete(out))
  }

  fn finish_if_complete(out: &mut Output) -> bool {
    if out.printed || !out.is_complete() {
      return false;
    }
    out.printed = true;

    let m = &out.mode;
    let g = &out.geo;
    let mut s = StringOnStack::<512>::new();
    s.push("Name: ")
      .push(out.name.as_str())
      .push("\nDescription: ")
      .push(out.desc)
      .push("\nCurrent Mode: ")
      .push(m.width)
      .push("x")
      .push(m.height)
      .push("@")
      .push(m.refresh / 1000)
      .push("\nPhysical Size: ")
      .push(g.physical_width)
      .push("x")
      .push(g.physical_height)
      .push("mm")
      .push("\nPosition: ")
      .push(g.x)
      .push(",")
      .push(g.y);
    if out.xdg_output_id != 0 {
      s.push("\nLogical Position: ")
        .push(out.logical.x)
        .push(",")
        .push(out.logical.y)
        .push("\nLogical Size: ")
        .push(out.logical.width)
        .push("x")
        .push(out.logical.height);
    }

    s.push("\nScale: ")
      .push(out.scale)
      .push("\nTransform: ")
      .push(transform_name(g.transform))
      .push("\nSubpixel: ")
      .push(subpixel_name(g.subpixel))
      .push("\n");

    write_stdout(s);
    true
  }
}

impl GlobalHandler for State {
  // bind globals matching interfaces we want. we bind to the minimum of the client's wanted
  // version and the server's advertised version
  fn on_global(
    &mut self,
    cx: &mut Ctx<'_>,
    name: u32,
    interface: &str,
    version: u32,
  ) -> Result<(), WireError> {
    match interface {
      "wl_compositor" => {
        let id = cx.alloc()?;
        bind(cx, name, interface, clamp_version(7, version), id)?;
        self.global.compositor_id = id;
      }
      "wl_output" => {
        if self.output_len >= MAX_OUTPUTS {
          write_stderr("Maximum outputs limit reached\n");
          return Ok(());
        }
        let id = cx.alloc()?;
        bind(cx, name, interface, clamp_version(4, version), id)?;
        self.outputs[self.output_len] = Output {
          output_id: id,
          ..Default::default()
        };
        self.output_len += 1;
      }
      "zxdg_output_manager_v1" => {
        let id = cx.alloc()?;
        // only `logical_position`/`logical_size` are wanted.
        bind(cx, name, interface, clamp_version(1, version), id)?;
        self.global.xdg_output_manager_id = id;
      }
      _ => (),
    }
    Ok(())
  }
}

impl EventHandler for State {
  fn handle_event(&mut self, _cx: &mut Ctx<'_>, msg: Message<'_>) -> Result<(), WireError> {
    let sender = msg.header.object_id;
    let Some(out) = self.outputs[..self.output_len]
      .iter_mut()
      .find(|out| out.output_id == sender || out.xdg_output_id == sender)
    else {
      return Ok(());
    };
    let completed = if out.output_id == sender {
      State::handle_output_event(out, &msg)?
    } else {
      State::handle_xdg_output_event(out, &msg)?
    };
    if completed {
      self.done_count += 1;
    }
    Ok(())
  }
}
