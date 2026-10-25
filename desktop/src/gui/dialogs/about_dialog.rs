use crate::gui::text;
use chrono::DateTime;
use egui::{Align2, Grid};
use unic_langid::LanguageIdentifier;

const VERGEN_UNKNOWN: &str = "VERGEN_IDEMPOTENT_OUTPUT";

/// About this distribution, with upstream attribution and license text.
pub fn show_about_dialog(locale: &LanguageIdentifier, egui_ctx: &egui::Context) -> bool {
    let mut keep_open = true;

    egui::Window::new(text(locale, "help-menu-about"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut keep_open)
        .show(egui_ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(crate::branding::APP_NAME);
                ui.label("基于 Ruffle 的独立游戏客户端。");
                ui.label("提供卡布西游、洛克王国登录接入及桌面体验改进。");
                ui.label("讨论 QQ 群：1075919913");
                ui.add_space(8.0);
                Grid::new("about_ruffle_version_info")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("维护者");
                        ui.label(crate::branding::MAINTAINER);
                        ui.end_row();
                        ui.label("源项目");
                        ui.hyperlink_to("Ruffle", "https://github.com/ruffle-rs/ruffle");
                        ui.end_row();

                        ui.label(text(locale, "about-ruffle-version"));
                        ui.label(env!("CARGO_PKG_VERSION"));
                        ui.end_row();

                        ui.label(text(locale, "about-ruffle-channel"));
                        ui.label(env!("CFG_RELEASE_CHANNEL"));
                        ui.end_row();

                        let build_time = env!("VERGEN_BUILD_TIMESTAMP");
                        if build_time != VERGEN_UNKNOWN {
                            ui.label(text(locale, "about-ruffle-build-time"));
                            ui.label(
                                DateTime::parse_from_rfc3339(build_time)
                                    .map(|t| t.format("%c").to_string())
                                    .unwrap_or_else(|_| build_time.to_string()),
                            );
                            ui.end_row();
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("源项目 Ruffle", "https://ruffle.rs");
                    ui.hyperlink_to("赞助源项目 Ruffle", "https://opencollective.com/ruffle/");
                });
                ui.add_space(8.0);
                ui.label("本客户端版权归“傲娇小霖霖”所有。");
                ui.label("Ruffle 原始代码版权归 Ruffle LLC 及其贡献者所有。");
                ui.label("源项目采用 MIT 或 Apache-2.0 许可。本版为独立修改版本。");
                ui.collapsing("开源许可与版权声明", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            ui.set_max_width(480.0);
                            ui.label(include_str!("../../../../LICENSE.md"));
                        });
                });
            })
        });

    keep_open
}
