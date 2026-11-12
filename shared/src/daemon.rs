use wllib::dispatch::{EventHandler, dispatch_once};
use wllib::error::WireError;
use wllib::registry::{GlobalHandler, crawl};
use wllib::transport::Connection;

use crate::connect_wayland;

pub fn daemon_runner<S, F, E>(mut state: S, setup: F) -> Result<(), E>
where
  S: GlobalHandler + EventHandler,
  F: FnOnce(&mut Connection, &mut S) -> Result<(), E>,
  E: From<WireError>,
{
  let mut conn = connect_wayland()?;
  crawl(&mut conn, &mut state)?;
  setup(&mut conn, &mut state)?;

  loop {
    match dispatch_once(&mut conn, &mut state) {
      Ok(()) => {}
      Err(WireError::ConnectionClosed) => break,
      Err(e) => return Err(e.into()),
    }
  }
  Ok(())
}
