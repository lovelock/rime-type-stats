//! 菜单栏入口与事件循环。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::icons::{menu_bar_title, ICON_RGBA};
use crate::store::Store;
use crate::sync::{sync_once, SyncState};

/// 同步线程唤醒 UI 的事件。
#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    /// 已同步完毕，请刷新菜单栏数字
    Synced,
}

const SYNC_INTERVAL: Duration = Duration::from_secs(60);

fn open_in_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    let _ = std::process::Command::new("open").arg(url).spawn();
}

fn open_data_dir(dir: &PathBuf) {
    let _ = std::process::Command::new("open").arg(dir).spawn();
}

fn build_icon() -> Icon {
    let (w, h, rgba) = ICON_RGBA;
    Icon::from_rgba(rgba.to_vec(), w, h).expect("内嵌图标必须是合法 RGBA")
}

/// 菜单栏标题所用的今日字数。
fn today_chars(state: &SyncState) -> u64 {
    let (days, _, _) = state.snapshot();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    days.get(&today).map(|d| d.characters).unwrap_or(0)
}

/// 运行菜单栏应用，永不返回。
pub fn run_app(rime_dir: PathBuf, data_dir: PathBuf, state: SyncState, port: u16) -> ! {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    // 后台同步线程：立即同步一次，之后按间隔循环
    {
        let state = state.clone();
        let proxy = proxy.clone();
        let rime_dir = rime_dir.clone();
        let data_dir = data_dir.clone();
        std::thread::spawn(move || {
            let Ok(store) = Store::open(&data_dir) else {
                return;
            };
            loop {
                let outcome = sync_once(&rime_dir, &store);
                state.replace(outcome);
                let _ = proxy.send_event(UserEvent::Synced);
                std::thread::sleep(SYNC_INTERVAL);
            }
        });
    }

    let mut tray: Option<TrayIcon> = None;
    let mut menu_open: Option<MenuItem> = None;
    let mut menu_sync: Option<MenuItem> = None;
    let mut menu_data_dir: Option<MenuItem> = None;
    let mut menu_quit: Option<MenuItem> = None;
    let mut next_sync = Instant::now() + SYNC_INTERVAL;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_sync);

        match event {
            Event::NewEvents(cause) => {
                // StartCause 为 non_exhaustive，用 matches! 判断初始化与定时到达
                let timer_fired = matches!(
                    cause,
                    StartCause::Init | StartCause::ResumeTimeReached { .. }
                );
                if timer_fired && Instant::now() >= next_sync {
                    next_sync = Instant::now() + SYNC_INTERVAL;
                    // 同步线程独立运行，这里只负责重排定时器并驱动标题刷新
                }

                // macOS 要求事件循环已开始运行后才能创建 TrayIcon
                if tray.is_none() {
                    let menu = Menu::new();
                    let open = MenuItem::new("打开统计", true, None);
                    let sync_now = MenuItem::new("立即同步", true, None);
                    let data = MenuItem::new("打开数据目录", true, None);
                    let quit = MenuItem::new("退出", true, None);
                    let _ = menu.append(&open);
                    let _ = menu.append(&sync_now);
                    let _ = menu.append(&data);
                    let _ = menu.append(&quit);

                    let icon = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_icon_as_template(true)
                        .with_icon(build_icon())
                        .with_tooltip("Rime 打字统计")
                        .with_title(menu_bar_title(today_chars(&state)))
                        .build();

                    if let Ok(icon) = icon {
                        tray = Some(icon);
                        menu_open = Some(open);
                        menu_sync = Some(sync_now);
                        menu_data_dir = Some(data);
                        menu_quit = Some(quit);
                    }
                }
            }
            Event::MainEventsCleared => {
                // 刷新标题
                if let Some(icon) = tray.as_ref() {
                    icon.set_title(Some(menu_bar_title(today_chars(&state))));
                }

                // 托盘左键：打开网页
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
                        std::thread::spawn(move || {
                            if let Ok(store) = Store::open(&data_dir) {
                                state.replace(sync_once(&rime_dir, &store));
                            }
                        });
                    } else if Some(&id) == menu_data_dir.as_ref().map(|m| m.id()) {
                        open_data_dir(&data_dir);
                    }
                }
            }
            Event::UserEvent(UserEvent::Synced) => {
                // 标题在 MainEventsCleared 中统一刷新
            }
            Event::LoopDestroyed => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}
