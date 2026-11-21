use crate::cli::Config;
use crate::io::write_stderr;
use rustix::fd::AsFd;
use wllib::dispatch::EventHandler;
use wllib::error::WireError;
use wllib::protocols::{
  wl_compositor, wl_shm, wl_shm_pool, wl_surface, zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use wllib::registry::{GlobalHandler, bind, clamp_version};
use wllib::transport::{Connection, Ctx};
use wllib::wire::{Message, Writer};

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

  pub fn create_surfaces(&mut self, conn: &mut Connection) -> Result<(), WireError> {
    for i in 0..self.output_len {
      let surf_id = conn.alloc_id()?;
      let layer_surf_id = conn.alloc_id()?;
      let output_id = self.outputs[i].output_id;

      let mut surf_msg = Writer::<16>::new(
        self.global.compositor_id,
        wl_compositor::request::CREATE_SURFACE,
      );
      conn.send(surf_msg.u32(surf_id).finish()?, &[])?;

      let mut ls_msg = Writer::<256>::new(
        self.global.layer_shell_id,
        zwlr_layer_shell_v1::request::GET_LAYER_SURFACE,
      );
      ls_msg
        .u32(layer_surf_id)
        .u32(surf_id)
        .u32(output_id)
        .u32(zwlr_layer_shell_v1::layer::BACKGROUND)
        .str(self.config.namespace.as_str());
      conn.send(ls_msg.finish()?, &[])?;

      let mut anchor_msg =
        Writer::<16>::new(layer_surf_id, zwlr_layer_surface_v1::request::SET_ANCHOR);
      conn.send(
        anchor_msg
          .u32(zwlr_layer_surface_v1::anchor::ALL)
          .finish()?,
        &[],
      )?;

      let mut ex_msg = Writer::<16>::new(
        layer_surf_id,
        zwlr_layer_surface_v1::request::SET_EXCLUSIVE_ZONE,
      );
      conn.send(ex_msg.i32(-1).finish()?, &[])?;

      let mut commit_msg = Writer::<8>::new(surf_id, wl_surface::request::COMMIT);
      conn.send(commit_msg.finish()?, &[])?;

      self.outputs[i].wl_surface_id = surf_id;
      self.outputs[i].layer_surface_id = layer_surf_id;
    }
    Ok(())
  }

  // zwlr_layer_surface_v1::configure(serial, width, height): the compositor telling us the
  // size to render at. we ack it, render the wallpaper at that size, and attach/commit it.
  fn handle_layer_surface(
    cx: &mut Ctx<'_>,
    out: &mut Output,
    config: &Config,
    shm_id: u32,
    msg: Message<'_>,
  ) -> Result<(), WireError> {
    let mut r = msg.args();
    let serial = r.u32()?;
    let width = r.u32()?;
    let height = r.u32()?;
    r.finish()?;

    let layer_surface_id = out.layer_surface_id;
    let mut w_ack = Writer::<16>::new(
      layer_surface_id,
      zwlr_layer_surface_v1::request::ACK_CONFIGURE,
    );
    w_ack.u32(serial);
    cx.send(w_ack.finish()?, &[])?;

    if width == 0 || height == 0 || out.size == (width, height) {
      return Ok(());
    }

    let fd = match image::get_image_fd(width, height, config) {
      Ok(fd) => fd,
      Err(e) => {
        e.write_diagnostic();
        return Ok(());
      }
    };

    let pool_id = cx.alloc()?;
    let buffer_id = cx.alloc()?;
    let stride = (width * 4) as i32;

    let mut w_pool = Writer::<24>::new(shm_id, wl_shm::request::CREATE_POOL);
    w_pool.u32(pool_id);
    w_pool.i32((width * height * 4) as i32);
    cx.send(w_pool.finish()?, &[fd.as_fd()])?;

    let mut w_buf = Writer::<32>::new(pool_id, wl_shm_pool::request::CREATE_BUFFER);
    w_buf.u32(buffer_id);
    w_buf.i32(0); // offset
    w_buf.i32(width as i32);
    w_buf.i32(height as i32);
    w_buf.i32(stride);
    w_buf.u32(wl_shm::format::ARGB8888);
    cx.send(w_buf.finish()?, &[])?;

    let mut w_destroy = Writer::<8>::new(pool_id, wl_shm_pool::request::DESTROY);
    cx.send(w_destroy.finish()?, &[])?;

    let wl_surface_id = out.wl_surface_id;

    let mut w_attach = Writer::<24>::new(wl_surface_id, wl_surface::request::ATTACH);
    w_attach.u32(buffer_id);
    w_attach.i32(0);
    w_attach.i32(0);
    cx.send(w_attach.finish()?, &[])?;

    let mut w_dmg = Writer::<24>::new(wl_surface_id, wl_surface::request::DAMAGE_BUFFER);
    w_dmg.i32(0);
    w_dmg.i32(0);
    w_dmg.i32(width as i32);
    w_dmg.i32(height as i32);
    cx.send(w_dmg.finish()?, &[])?;

    let mut w_commit = Writer::<8>::new(wl_surface_id, wl_surface::request::COMMIT);
    cx.send(w_commit.finish()?, &[])?;

    // Replace the surface content before releasing the previous buffer object.
    if out.buffer_id != 0 {
      let mut destroy = Writer::<8>::new(out.buffer_id, WL_BUFFER_DESTROY);
      cx.send(destroy.finish()?, &[])?;
    }
    out.buffer_id = buffer_id;
    out.size = (width, height);

    Ok(())
  }
}

impl GlobalHandler for State {
  fn on_global(
    &mut self,
    cx: &mut Ctx<'_>,
    name: u32,
    interface: &str,
    version: u32,
  ) -> Result<(), WireError> {
    let (wanted, slot) = match interface {
      "wl_compositor" => (7, &mut self.global.compositor_id),
      "wl_shm" => (2, &mut self.global.shm_id),
      "zwlr_layer_shell_v1" => (5, &mut self.global.layer_shell_id),
      "wl_output" => {
        if self.output_len >= MAX_OUTPUTS {
          write_stderr("Maximum outputs limit reached\n");
          return Ok(());
        }
        let id = cx.alloc()?;
        bind(cx, name, interface, clamp_version(4, version), id)?;
        self.outputs[self.output_len].output_id = id;
        self.output_len += 1;
        return Ok(());
      }
      _ => return Ok(()),
    };
    let id = cx.alloc()?;
    bind(cx, name, interface, clamp_version(wanted, version), id)?;
    *slot = id;
    Ok(())
  }
}

impl EventHandler for State {
  fn handle_event(&mut self, cx: &mut Ctx<'_>, msg: Message<'_>) -> Result<(), WireError> {
    if msg.header.opcode == zwlr_layer_surface_v1::event::CONFIGURE
      && let Some(out) = self.outputs[..self.output_len]
        .iter_mut()
        .find(|out| out.layer_surface_id == msg.header.object_id)
    {
      Self::handle_layer_surface(cx, out, &self.config, self.global.shm_id, msg)?;
    }
    Ok(())
  }
}
