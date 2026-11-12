use rustix::fd::AsFd;
use shared::error::write_wire_diagnostic;
use shared::io::write_stderr;
use shared::{ConnectionExt, CtxExt};
use wllib::dispatch::EventHandler;
use wllib::error::WireError;
use wllib::protocols::{zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1};
use wllib::registry::{GlobalHandler, bind, clamp_version};
use wllib::transport::{Connection, Ctx};
use wllib::wire::{Message, Writer};

use crate::error::AppError;
use crate::gamma;

const MAX_OUTPUTS: usize = 8;

// tracks registered object ids
#[derive(Default)]
pub struct Global {
  // wl_compositor global id
  pub compositor_id: u32,
  // zwlr_layer_shell_v1 global id
  pub gamma_manager_id: u32,
}

// abstration around wl_output
#[derive(Default)]
pub struct Output {
  pub output_id: u32,
  pub gamma_control_id: u32,
}

// config
pub struct Config {
  pub temp: f64,
  // input levels: remaps [level_black, level_white] to [0, 1] before any other ramp op
  pub level_black: f64,
  pub level_white: f64,
  // power-curve exponent. 1.0 is a no-op
  pub gamma: f64,
  // per-channel gain (multiplicative) and offset (additive), applied after the color-temperature
  // factor. also doubles as a manual per-channel black/white point:
  // (gain = 1/(white-black), offset = -black*gain is the same affine transform)
  pub gain_r: f64,
  pub gain_g: f64,
  pub gain_b: f64,
  pub offset_r: f64,
  pub offset_g: f64,
  pub offset_b: f64,
  // sigmoid contrast strength, roughly -1.0..1.0. 0.0 is a no-op
  pub contrast: f64,
  // final multiplicative scale, applied after contrast. 1.0 is a no-op
  pub brightness: f64,
  // quantizes to this many discrete output levels. < 2 disables it
  pub posterize_levels: u32,
  // inverts values above the threshold, applied after posterize. 1.0 is a no-op
  pub solarize_value: f64,
  // full inversion
  pub invert: bool,
  // zeros out an entire channel's ramp, bypassing the rest of the pipeline for that channel
  pub disable_r: bool,
  pub disable_g: bool,
  pub disable_b: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      temp: 6500.0,
      level_black: 0.0,
      level_white: 1.0,
      gamma: 1.0,
      contrast: 0.0,
      brightness: 1.0,
      gain_r: 1.0,
      gain_g: 1.0,
      gain_b: 1.0,
      offset_r: 0.0,
      offset_g: 0.0,
      offset_b: 0.0,
      posterize_levels: 0,
      solarize_value: 1.0,
      invert: false,
      disable_r: false,
      disable_g: false,
      disable_b: false,
    }
  }
}

pub struct State {
  pub global: Global,
  pub outputs: [Output; MAX_OUTPUTS],
  pub output_len: usize,
  pub config: Config,
}

impl State {
  pub fn init(config: Config) -> Self {
    Self {
      global: Global::default(),
      outputs: core::array::from_fn(|_| Output::default()),
      output_len: 0,
      config,
    }
  }

  pub fn setup_outputs<const N: usize>(
    &mut self,
    conn: &mut Connection<N>,
  ) -> Result<(), AppError> {
    // setup layer surfaces and gamma control for all monitors.
    for i in 0..self.output_len {
      // alloc ids for the new gamma control objects.
      let gamma_ctrl_id = conn.alloc_id()?;
      let output_id = self.outputs[i].output_id;

      let mut w = Writer::<32>::new(
        self.global.gamma_manager_id,
        zwlr_gamma_control_manager_v1::request::GET_GAMMA_CONTROL,
      );
      w.u32(gamma_ctrl_id).u32(output_id);

      // request gamma control
      conn.send_logged(w.finish()?, &[]);

      // store the allocated ids back into our Output instance
      self.outputs[i].gamma_control_id = gamma_ctrl_id;
    }
    Ok(())
  }

  // zwlr_gamma_control_v1::gamma_size(size)
  fn handle_gamma_size(
    &self,
    conn: &mut Ctx<'_>,
    out: &Output,
    config: &Config,
    msg: Message<'_>,
  ) -> Result<(), AppError> {
    let mut r = msg.args();

    let size = r.u32()? as usize;

    // get the fd containing the scaled image data
    let fd = gamma::get_gamma_table_fd(size, config)?;
    let gamma_control_id = out.gamma_control_id;

    let mut w = Writer::<32>::new(gamma_control_id, zwlr_gamma_control_v1::request::SET_GAMMA);

    // set gamma table
    conn.send_logged(w.finish()?, &[fd.as_fd()]);
    Ok(())
  }
}

impl GlobalHandler for State {
  // bind globals matching interfaces we want. we bind to the minimum of the client's wanted
  // version and the server's advertised version
  fn on_global(
    &mut self,
    conn: &mut Ctx<'_>,
    name: u32,
    interface: &str,
    version: u32,
  ) -> Result<(), WireError> {
    match interface {
      "wl_compositor" => {
        let id = conn.alloc()?;
        match bind(conn, name, interface, clamp_version(7, version), id) {
          Ok(()) => self.global.compositor_id = id,
          Err(e) => write_wire_diagnostic("[rustemp]", &e),
        }
      }
      "zwlr_gamma_control_manager_v1" => {
        let id = conn.alloc()?;
        match bind(conn, name, interface, clamp_version(1, version), id) {
          Ok(()) => self.global.gamma_manager_id = id,
          Err(e) => write_wire_diagnostic("[rustemp]", &e),
        }
      }
      "wl_output" => {
        if self.output_len >= MAX_OUTPUTS {
          write_stderr("Maximum outputs limit reached\n");
          // FIXME: needs new error variant
          return Err(WireError::MessageTooLarge);
        }
        let id = conn.alloc()?;
        match bind(conn, name, interface, clamp_version(4, version), id) {
          Ok(()) => {
            self.outputs[self.output_len] = Output {
              output_id: id,
              ..Default::default()
            };
            self.output_len += 1;
          }
          Err(e) => write_wire_diagnostic("[rustemp]", &e),
        }
      }
      _ => (),
    }
    Ok(())
  }
}

impl EventHandler for State {
  fn handle_event(&mut self, conn: &mut Ctx<'_>, msg: Message<'_>) -> Result<(), WireError> {
    for out in &self.outputs {
      if out.gamma_control_id != msg.header.object_id {
        continue;
      }
      if msg.header.opcode != zwlr_gamma_control_v1::event::FAILED {
        if let Err(e) = self.handle_gamma_size(conn, out, &self.config, msg) {
          e.write_diagnostic();
        }
      }
    }
    Ok(())
  }
}
