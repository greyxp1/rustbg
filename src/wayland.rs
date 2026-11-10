use crate::cli::Config;
use crate::error::write_wire_diagnostic;
use crate::io::write_stderr;
use rustix::fd::OwnedFd;
use wllib::dispatch::EventHandler;
use wllib::protocols::{
  wl_compositor, wl_shm, wl_shm_pool, wl_surface, zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use wllib::registry::{GlobalHandler, bind, clamp_version};
use wllib::transport::Connection;
use wllib::wire::{Message, read_u32};

use crate::image;

const MAX_OUTPUTS: usize = 8;
const WL_BUFFER_DESTROY: u16 = 0;

#[derive(Default)]
struct Global {
  compositor_id: u32,
  shm_id: u32,
  layer_shell_id: u32,
}

#[derive(Default)]
struct Output {
  output_id: u32,
  wl_surface_id: u32,
  layer_surface_id: u32,
  buffer_id: u32,
  size: (u32, u32),
}

pub struct State {
  global: Global,
  outputs: [Output; MAX_OUTPUTS],
  output_len: usize,
  config: Config,
}
impl State {
  #[must_use]
  pub fn init(config: Config) -> Self {
    Self {
      global: Global::default(),
      outputs: core::array::from_fn(|_| Output::default()),
      output_len: 0,
      config,
    }
  }

  pub fn create_surfaces(&mut self, conn: &mut Connection) {
    for i in 0..self.output_len {
      let surf_id = conn.alloc_id();
      let layer_surf_id = conn.alloc_id();
      let output_id = self.outputs[i].output_id;

      let mut surf_msg = Message::new(
        self.global.compositor_id,
        wl_compositor::request::CREATE_SURFACE,
      );
      surf_msg.write_u32(surf_id);
      conn.send_logged(&surf_msg, None);

      let mut ls_msg = Message::new(
        self.global.layer_shell_id,
        zwlr_layer_shell_v1::request::GET_LAYER_SURFACE,
      );
      ls_msg.write_u32(layer_surf_id);
      ls_msg.write_u32(surf_id);
      ls_msg.write_u32(output_id);
      ls_msg.write_u32(zwlr_layer_shell_v1::layer::BACKGROUND);
      ls_msg.write_cstr(self.config.namespace.as_c_str());
      conn.send_logged(&ls_msg, None);

      let mut anchor_msg = Message::new(layer_surf_id, zwlr_layer_surface_v1::request::SET_ANCHOR);
      anchor_msg.write_u32(zwlr_layer_surface_v1::anchor::ALL);
      conn.send_logged(&anchor_msg, None);

      let mut ex_msg = Message::new(
        layer_surf_id,
        zwlr_layer_surface_v1::request::SET_EXCLUSIVE_ZONE,
      );
      ex_msg.write_i32(-1);
      conn.send_logged(&ex_msg, None);

      conn.send_logged(&Message::new(surf_id, wl_surface::request::COMMIT), None);

      self.outputs[i].wl_surface_id = surf_id;
      self.outputs[i].layer_surface_id = layer_surf_id;
    }
  }

  fn handle_layer_surface(
    conn: &mut Connection,
    out: &mut Output,
    config: &Config,
    shm_id: u32,
    opcode: u16,
    data: &[u8],
  ) {
    if opcode != zwlr_layer_surface_v1::event::CONFIGURE {
      return;
    }
    let serial = read_u32(data, 0);
    let width = read_u32(data, 4);
    let height = read_u32(data, 8);

    let layer_surface_id = out.layer_surface_id;
    let mut ack = Message::new(
      layer_surface_id,
      zwlr_layer_surface_v1::request::ACK_CONFIGURE,
    );
    ack.write_u32(serial);
    conn.send_logged(&ack, None);

    if width == 0 || height == 0 || out.size == (width, height) {
      return;
    }

    let fd = match image::get_image_fd(width, height, config) {
      Ok(fd) => fd,
      Err(e) => {
        e.write_diagnostic();
        return;
      }
    };

    let pool_id = conn.alloc_id();
    let buffer_id = conn.alloc_id();
    let stride = (width * 4) as i32;

    let mut pool_msg = Message::new(shm_id, wl_shm::request::CREATE_POOL);
    pool_msg.write_u32(pool_id);
    pool_msg.write_i32((width * height * 4) as i32);
    conn.send_logged(&pool_msg, Some(&fd));

    let mut buf_msg = Message::new(pool_id, wl_shm_pool::request::CREATE_BUFFER);
    buf_msg.write_u32(buffer_id);
    buf_msg.write_i32(0); // offset
    buf_msg.write_i32(width as i32);
    buf_msg.write_i32(height as i32);
    buf_msg.write_i32(stride);
    buf_msg.write_u32(wl_shm::format::ARGB8888);
    conn.send_logged(&buf_msg, None);

    conn.send_logged(&Message::new(pool_id, wl_shm_pool::request::DESTROY), None);

    let wl_surface_id = out.wl_surface_id;

    let mut attach = Message::new(wl_surface_id, wl_surface::request::ATTACH);
    attach.write_u32(buffer_id);
    attach.write_i32(0);
    attach.write_i32(0);
    conn.send_logged(&attach, None);

    let mut dmg = Message::new(wl_surface_id, wl_surface::request::DAMAGE_BUFFER);
    dmg.write_i32(0);
    dmg.write_i32(0);
    dmg.write_i32(width as i32);
    dmg.write_i32(height as i32);
    conn.send_logged(&dmg, None);

    conn.send_logged(
      &Message::new(wl_surface_id, wl_surface::request::COMMIT),
      None,
    );

    // Replace the surface content before releasing the previous buffer object.
    if out.buffer_id != 0 {
      conn.send_logged(&Message::new(out.buffer_id, WL_BUFFER_DESTROY), None);
    }
    out.buffer_id = buffer_id;
    out.size = (width, height);
  }
}
impl GlobalHandler for State {
  fn on_global(&mut self, conn: &mut Connection, name: u32, interface: &str, version: u32) {
    let (wanted, slot) = match interface {
      "wl_compositor" => (7, &mut self.global.compositor_id),
      "wl_shm" => (2, &mut self.global.shm_id),
      "zwlr_layer_shell_v1" => (5, &mut self.global.layer_shell_id),
      "wl_output" => {
        if self.output_len >= MAX_OUTPUTS {
          write_stderr("Maximum outputs limit reached\n");
          return;
        }
        let id = conn.alloc_id();
        match bind(conn, name, interface, clamp_version(4, version), id) {
          Ok(()) => {
            self.outputs[self.output_len].output_id = id;
            self.output_len += 1;
          }
          Err(e) => write_wire_diagnostic("[rustbg]", &e),
        }
        return;
      }
      _ => return,
    };
    let id = conn.alloc_id();
    match bind(conn, name, interface, clamp_version(wanted, version), id) {
      Ok(()) => *slot = id,
      Err(e) => write_wire_diagnostic("[rustbg]", &e),
    }
  }
}
impl EventHandler for State {
  fn handle_event(&mut self, conn: &mut Connection, sender: u32, opcode: u16, data: &[u8]) {
    self.outputs.iter_mut().for_each(|out| {
      if out.layer_surface_id != sender {
        return;
      }
      State::handle_layer_surface(conn, out, &self.config, self.global.shm_id, opcode, data);
    });
  }
}

trait ConnectionExt {
  fn send_logged(&self, msg: &Message, fd: Option<&OwnedFd>);
}

impl ConnectionExt for Connection {
  fn send_logged(&self, msg: &Message, fd: Option<&OwnedFd>) {
    if let Err(e) = self.send(msg, fd) {
      write_wire_diagnostic("wllib", &e);
    }
  }
}
