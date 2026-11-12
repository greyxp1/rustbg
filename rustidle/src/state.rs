use shared::io::write_stderr;
use wllib::dispatch::EventHandler;
use wllib::error::WireError;
use wllib::protocols::{ext_idle_notification_v1, ext_idle_notifier_v1};
use wllib::registry::{GlobalHandler, bind, clamp_version};
use wllib::transport::{Connection, Ctx};
use wllib::wire::{Message, Writer};

use crate::config::{Config, Entry, MAX_ENTRIES};
use crate::error::AppError;

// tracks registered object ids
#[derive(Default)]
pub struct Global {
  // wl_seat global id
  pub seat_id: u32,
  // ext_idle_notifier global id
  pub idle_notifier_id: u32,
}

#[derive(Default)]
pub struct Notification {
  pub id: u32,
  pub entry: Entry,
}

pub struct State {
  pub global: Global,
  pub config: Config,
  pub notifications: [Notification; MAX_ENTRIES],
}

impl State {
  #[must_use]
  pub fn init(config: Config) -> Self {
    let mut notifications = core::array::from_fn(|_| Notification::default());
    (0..config.entry_len).for_each(|i| notifications[i].entry = config.entries[i]);

    Self {
      global: Global::default(),
      config,
      notifications,
    }
  }

  /// Request idle notification objects for configured entries.
  pub fn create_notifications<const N: usize>(
    &mut self,
    conn: &mut Connection<N>,
  ) -> Result<(), AppError> {
    for i in 0..self.config.entry_len {
      let notif = &mut self.notifications[i];
      let id = conn.alloc_id()?;
      notif.id = id;

      let mut msg = Writer::<64>::new(
        self.global.idle_notifier_id,
        ext_idle_notifier_v1::request::GET_IDLE_NOTIFICATION,
      );
      msg
        .u32(id)
        .u32(notif.entry.timeout_ms)
        .u32(self.global.seat_id);
      conn.send(msg.finish()?, &[])?;
    }
    Ok(())
  }

  /// Destroy active idle notification objects.
  pub fn destroy_notifications<const N: usize>(
    &mut self,
    conn: &mut Connection<N>,
  ) -> Result<(), AppError> {
    for i in 0..self.config.entry_len {
      let notif = &mut self.notifications[i];
      if notif.id != 0 {
        let mut msg = Writer::<64>::new(notif.id, ext_idle_notification_v1::request::DESTROY);
        conn.send(msg.finish()?, &[])?;
        notif.id = 0;
      }
    }
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
      "wl_seat" => {
        if self.global.seat_id != 0 {
          write_stderr("failed to bind seat");
          return Ok(());
        }
        let id = conn.alloc()?;
        bind(conn, name, interface, clamp_version(1, version), id)?;
        self.global.seat_id = id;
      }
      "ext_idle_notifier_v1" => {
        let id = conn.alloc()?;
        bind(conn, name, interface, clamp_version(2, version), id)?;
        self.global.idle_notifier_id = id;
      }
      _ => {}
    }
    Ok(())
  }
}

impl EventHandler for State {
  fn handle_event(&mut self, _conn: &mut Ctx<'_>, msg: Message<'_>) -> Result<(), WireError> {
    for notif in &self.notifications {
      if notif.id != msg.header.object_id {
        continue;
      }
      match msg.header.opcode {
        ext_idle_notification_v1::event::IDLED => {
          if let Some(argv) = notif.entry.idle_argv
            && let Err(e) = self.config.spawn(argv)
          {
            e.write_diagnostic();
          }
          return Ok(());
        }
        ext_idle_notification_v1::event::RESUMED => {
          if let Some(argv) = notif.entry.resume_argv
            && let Err(e) = self.config.spawn(argv)
          {
            e.write_diagnostic();
          }
          return Ok(());
        }
        _ => (),
      }
    }
    Ok(())
  }
}
