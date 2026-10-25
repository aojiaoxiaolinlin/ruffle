//! User-facing identity of this independently maintained desktop distribution.
pub const APP_NAME: &str = "傲娇游戏盒";
pub const MAINTAINER: &str = "傲娇小霖霖";

pub fn library_title() -> String {
    format!("{APP_NAME} - 游戏列表")
}
