//! 菜单栏 / 系统托盘入口与事件循环。
//!
//! macOS 用状态栏标题显示今日字数；Linux 走 AppIndicator，只支持菜单，
//! 因此今日字数放在菜单首项。

use std::path::{Path, PathBuf};
use std::time::Duration;

use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
#[cfg(target_os = "macos")]
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

#[cfg(target_os = "macos")]
use crate::icons::menu_bar_title;
#[cfg(not(target_os = "macos"))]
use crate::icons::today_menu_text;
use crate::icons::ICON_RGBA;
use crate::store::Store;
use crate::sync::{sync_once, SyncState};

/// 同步线程唤醒 UI 的事件。
#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    /// 已同步完毕，请刷新菜单栏数字
    Synced,
}

const SYNC_INTERVAL: Duration = Duration::from_secs(60);

#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(not(target_os = "macos"))]
const OPENER: &str = "xdg-open";

fn open_in_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    let _ = std::process::Command::new(OPENER).arg(url).spawn();
}

fn open_data_dir(dir: &Path) {
    let _ = std::process::Command::new(OPENER).arg(dir).spawn();
}

fn build_icon() -> Icon {
    let (w, h, rgba) = ICON_RGBA;
    Icon::from_rgba(rgba.to_vec(), w, h).expect("内嵌图标必须是合法 RGBA")
}

/// 托盘/菜单栏显示的今日字数。
fn today_chars(state: &SyncState) -> u64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    state.characters_on(&today)
}

/// 运行一次同步，成功后通知 UI 刷新显示。
fn sync_and_notify(rime_dir: &Path, data_dir: &Path, state: &SyncState, notify: impl Fn()) {
    if let Ok(store) = Store::open(data_dir) {
        state.replace(sync_once(rime_dir, &store));
        notify();
    }
}

/// 运行托盘应用，永不返回。
pub fn run_app(rime_dir: PathBuf, data_dir: PathBuf, state: SyncState, port: u16) -> ! {
    #[cfg(target_os = "macos")]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(not(target_os = "macos"))]
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    // 后台同步线程：启动时已经同步过一次，这里先等一个周期再开始
    {
        let state = state.clone();
        let proxy = proxy.clone();
        let rime_dir = rime_dir.clone();
        let data_dir = data_dir.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(SYNC_INTERVAL);
            sync_and_notify(&rime_dir, &data_dir, &state, || {
                let _ = proxy.send_event(UserEvent::Synced);
            });
        });
    }

    let mut tray: Option<TrayIcon> = None;
    let mut menu_open: Option<MenuItem> = None;
    let mut menu_sync: Option<MenuItem> = None;
    let mut menu_data_dir: Option<MenuItem> = None;
    let mut menu_quit: Option<MenuItem> = None;
    #[cfg(not(target_os = "macos"))]
    let mut menu_today: Option<MenuItem> = None;
    // 已写入菜单栏的标题。AppKit 的 setTitle 即使内容不变也会标记状态栏项为脏，
    // 触发重绘 → 产生事件 → 再次 setTitle，形成自持重绘循环（实测吃掉 97% CPU）。
    // 因此标题只在同步完成且文本确实变化时才更新。
    #[cfg(target_os = "macos")]
    let mut shown_title: Option<String> = None;
    // Linux 菜单首项同理：只在文本变化时 set_text，避免无意义重绘。
    #[cfg(not(target_os = "macos"))]
    let mut shown_menu: Option<String> = None;

    event_loop.run(move |event, _, control_flow| {
        // 完全阻塞等待：周期同步由后台线程自己 sleep 驱动，事件循环不需要定时器
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(_) => {
                // 事件循环开始运行后才能创建托盘图标
                if tray.is_none() {
                    let menu = Menu::new();

                    #[cfg(not(target_os = "macos"))]
                    let today = MenuItem::new(today_menu_text(today_chars(&state)), false, None);
                    #[cfg(not(target_os = "macos"))]
                    let _ = menu.append(&today);

                    let open = MenuItem::new("打开统计", true, None);
                    let sync_now = MenuItem::new("立即同步", true, None);
                    let data = MenuItem::new("打开数据目录", true, None);
                    let quit = MenuItem::new("退出", true, None);
                    let _ = menu.append(&open);
                    let _ = menu.append(&sync_now);
                    let _ = menu.append(&data);
                    let _ = menu.append(&quit);

                    #[cfg(target_os = "macos")]
                    let initial_title = menu_bar_title(today_chars(&state));

                    #[cfg(target_os = "macos")]
                    let builder = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_icon_as_template(true)
                        .with_icon(build_icon())
                        .with_tooltip("Rime 打字统计")
                        .with_title(&initial_title);
                    #[cfg(not(target_os = "macos"))]
                    let builder = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_icon(build_icon())
                        .with_tooltip("Rime 打字统计");

                    let icon = builder.build();
                    if let Err(err) = &icon {
                        eprintln!("创建托盘图标失败：{err}");
                        // Linux 上没有托盘就没有入口，退出让 systemd 按 Restart 策略重试。
                        #[cfg(not(target_os = "macos"))]
                        std::process::exit(1);
                    }

                    if let Ok(icon) = icon {
                        tray = Some(icon);
                        #[cfg(target_os = "macos")]
                        {
                            shown_title = Some(initial_title);
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            shown_menu = Some(today_menu_text(today_chars(&state)));
                            menu_today = Some(today);
                        }
                        menu_open = Some(open);
                        menu_sync = Some(sync_now);
                        menu_data_dir = Some(data);
                        menu_quit = Some(quit);
                    }
                }
            }
            Event::MainEventsCleared => {
                // 托盘左键：打开网页。Linux 的 AppIndicator 不产生点击事件。
                #[cfg(target_os = "macos")]
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        open_in_browser(port);
                    }
                }

                // 菜单项
                while let Ok(MenuEvent { id }) = MenuEvent::receiver().try_recv() {
                    if Some(&id) == menu_quit.as_ref().map(|m| m.id()) {
                        *control_flow = ControlFlow::Exit;
                    } else if Some(&id) == menu_open.as_ref().map(|m| m.id()) {
                        open_in_browser(port);
                    } else if Some(&id) == menu_sync.as_ref().map(|m| m.id()) {
                        let rime_dir = rime_dir.clone();
                        let data_dir = data_dir.clone();
                        let state = state.clone();
                        let proxy = proxy.clone();
                        std::thread::spawn(move || {
                            sync_and_notify(&rime_dir, &data_dir, &state, || {
                                let _ = proxy.send_event(UserEvent::Synced);
                            });
                        });
                    } else if Some(&id) == menu_data_dir.as_ref().map(|m| m.id()) {
                        open_data_dir(&data_dir);
                    }
                }
            }
            Event::UserEvent(UserEvent::Synced) => {
                // 同步完成后才刷新显示。这是唯一的更新时机：
                // macOS 上若在 MainEventsCleared 里无条件 setTitle，会形成
                // 重绘 → 事件 → 再重绘 的自持循环（实测占满 97% 单核）。
                #[cfg(target_os = "macos")]
                if let Some(icon) = tray.as_ref() {
                    let title = menu_bar_title(today_chars(&state));
                    if shown_title.as_deref() != Some(title.as_str()) {
                        icon.set_title(Some(title.clone()));
                        shown_title = Some(title);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                if let Some(item) = menu_today.as_ref() {
                    let text = today_menu_text(today_chars(&state));
                    if shown_menu.as_deref() != Some(text.as_str()) {
                        item.set_text(&text);
                        shown_menu = Some(text);
                    }
                }
            }
            Event::LoopDestroyed => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_and_notify_calls_notifier_after_successful_sync() {
        let dir =
            std::env::temp_dir().join(format!("rime-type-stats-tray-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let state = SyncState::new(Default::default());
        let calls = std::sync::atomic::AtomicUsize::new(0);

        sync_and_notify(&dir, &dir, &state, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "同步成功后应通知 UI 刷新"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
