use std::path::PathBuf;
use std::time::{Duration, Instant};

use rfd::{MessageButtons, MessageDialogResult};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::status::{Lang, Pending, Snapshot, is_running, labels, menu_state};
use crate::worker::{self, Action, Update};

const ICON_PNG: &[u8] = include_bytes!("../assets/icon-64.png");
const ACTION_ERROR_TTL: Duration = Duration::from_secs(15);
const PENDING_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy)]
enum Confirmable {
    Quit,
    Stop,
}

enum UserEvent {
    Menu(MenuEvent),
    Worker(Update),
    Confirmed(Confirmable, MessageDialogResult),
}

struct Items {
    status: MenuItem,
    open: MenuItem,
    restart: MenuItem,
    stop: MenuItem,
    start: MenuItem,
    quit: MenuItem,
}

impl Items {
    fn new(lang: Lang) -> Self {
        let l = labels(lang);
        Self {
            status: MenuItem::new("SWING", false, None),
            open: MenuItem::new(l.open, false, None),
            restart: MenuItem::new(l.restart, false, None),
            stop: MenuItem::new(l.stop, false, None),
            start: MenuItem::new(l.start, false, None),
            quit: MenuItem::new(l.quit, true, None),
        }
    }

    fn menu(&self) -> Menu {
        let menu = Menu::new();
        menu.append_items(&[
            &self.status,
            &PredefinedMenuItem::separator(),
            &self.open,
            &self.restart,
            &self.stop,
            &self.start,
            &PredefinedMenuItem::separator(),
            &self.quit,
        ])
        .expect("building the tray menu");
        menu
    }

    fn action(&self, id: &MenuId) -> Option<Action> {
        [
            (&self.open, Action::Open),
            (&self.restart, Action::Restart),
            (&self.stop, Action::Stop),
            (&self.start, Action::Start),
        ]
        .into_iter()
        .find(|(item, _)| item.id() == id)
        .map(|(_, action)| action)
    }
}

// Shown from another thread so the event loop keeps handling the menu and status updates meanwhile.
fn confirm(lang: Lang, what: Confirmable, proxy: EventLoopProxy<UserEvent>) {
    let (description, buttons) = match what {
        Confirmable::Quit => (labels(lang).quit_confirm, MessageButtons::YesNoCancel),
        Confirmable::Stop => (labels(lang).stop_confirm, MessageButtons::YesNo),
    };
    std::thread::spawn(move || {
        let answer = rfd::MessageDialog::new()
            .set_title("SWING")
            .set_description(description)
            .set_buttons(buttons)
            .show();
        let _ = proxy.send_event(UserEvent::Confirmed(what, answer));
    });
}

fn decode_png(bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("decoding the tray icon");
    let mut buf = vec![0; reader.output_buffer_size().expect("tray icon size")];
    let info = reader.next_frame(&mut buf).expect("decoding the tray icon");
    buf.truncate(info.buffer_size());
    (buf, info.width, info.height)
}

fn icons() -> (Icon, Icon) {
    let (rgba, width, height) = decode_png(ICON_PNG);
    let gray: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|px| {
            let luma = (0.299 * f32::from(px[0])
                + 0.587 * f32::from(px[1])
                + 0.114 * f32::from(px[2])) as u8;
            [luma, luma, luma, px[3] / 2]
        })
        .collect();
    (
        Icon::from_rgba(rgba, width, height).expect("building the tray icon"),
        Icon::from_rgba(gray, width, height).expect("building the tray icon"),
    )
}

struct View {
    lang: Lang,
    items: Items,
    tray: Option<TrayIcon>,
    active_icon: Icon,
    inactive_icon: Icon,
    snapshot: Option<Snapshot>,
    pending: Option<(Pending, Instant)>,
    action_error: Option<(String, Instant)>,
    shown_active: Option<bool>,
}

impl View {
    fn set_pending(&mut self, pending: Pending) {
        self.pending = Some((pending, Instant::now()));
        self.action_error = None;
        self.render();
    }

    fn update(&mut self, update: Update) {
        match update {
            Update::Snapshot(snapshot) => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|(p, _)| p.settled_by(&snapshot.status))
                {
                    self.pending = None;
                }
                self.snapshot = Some(snapshot);
            }
            Update::AutoStarting => self.pending = Some((Pending::Starting, Instant::now())),
            Update::ActionDone => self.action_error = None,
            Update::ActionFailed(e) => {
                self.pending = None;
                self.action_error = Some((e, Instant::now()));
            }
            Update::ReadyToQuit | Update::ServiceRemoved => {}
        }
        self.render();
    }

    fn render(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(p, at)| *p != Pending::Quitting && at.elapsed() >= PENDING_TIMEOUT)
        {
            self.pending = None;
        }
        let (Some(tray), Some(snapshot)) = (&self.tray, &self.snapshot) else {
            return;
        };
        let error = self
            .action_error
            .as_ref()
            .filter(|(_, at)| at.elapsed() < ACTION_ERROR_TTL)
            .map(|(e, _)| e.as_str());
        let state = menu_state(
            snapshot,
            self.pending.as_ref().map(|(p, _)| p),
            error,
            self.lang,
        );
        let items = &self.items;
        items.status.set_text(&state.status_text);
        items.open.set_enabled(state.open);
        items.restart.set_enabled(state.restart);
        items.stop.set_enabled(state.stop);
        items.start.set_enabled(state.start);
        items.quit.set_enabled(state.quit);
        let _ = tray.set_tooltip(Some(&state.status_text));
        if self.shown_active != Some(state.active_icon) {
            let icon = if state.active_icon {
                &self.active_icon
            } else {
                &self.inactive_icon
            };
            let _ = tray.set_icon(Some(icon.clone()));
            self.shown_active = Some(state.active_icon);
        }
    }
}

pub fn run(config_path: Option<PathBuf>, _lock: Option<std::fs::File>) -> ! {
    let lang = Lang::from_locale(sys_locale::get_locale().as_deref());

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let (actions, receiver) = tokio::sync::mpsc::unbounded_channel();
    let dialog_proxy = event_loop.create_proxy();
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("starting the tokio runtime")
            .block_on(worker::run(config_path, receiver, move |update| {
                let _ = proxy.send_event(UserEvent::Worker(update));
            }));
    });

    let (active_icon, inactive_icon) = icons();
    let mut view = View {
        lang,
        items: Items::new(lang),
        tray: None,
        active_icon,
        inactive_icon,
        snapshot: None,
        pending: None,
        action_error: None,
        shown_active: None,
    };
    let mut confirming = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut quit = false;
        match event {
            Event::NewEvents(StartCause::Init) => {
                view.tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(view.items.menu()))
                        .with_icon(view.inactive_icon.clone())
                        .with_tooltip("SWING")
                        .with_menu_on_left_click(true)
                        .build()
                        .expect("creating the tray icon"),
                );
                view.render();
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                let ask = if event.id == *view.items.quit.id() {
                    if is_running(view.snapshot.as_ref()) {
                        Some(Confirmable::Quit)
                    } else {
                        quit = true;
                        None
                    }
                } else {
                    match view.items.action(&event.id) {
                        Some(Action::Stop) => Some(Confirmable::Stop),
                        Some(action) => {
                            match action {
                                Action::Start => view.set_pending(Pending::Starting),
                                Action::Restart => {
                                    view.set_pending(Pending::restarting(view.snapshot.as_ref()))
                                }
                                _ => {}
                            }
                            let _ = actions.send(action);
                            None
                        }
                        None => None,
                    }
                };
                if let Some(what) = ask
                    && !confirming
                {
                    confirming = true;
                    confirm(lang, what, dialog_proxy.clone());
                }
            }
            Event::UserEvent(UserEvent::Confirmed(what, answer)) => {
                confirming = false;
                match (what, answer) {
                    (Confirmable::Quit, MessageDialogResult::Yes) => {
                        view.set_pending(Pending::Quitting);
                        let _ = actions.send(Action::StopAndQuit);
                    }
                    (Confirmable::Quit, MessageDialogResult::No) => quit = true,
                    (Confirmable::Stop, MessageDialogResult::Yes) => {
                        view.set_pending(Pending::Stopping);
                        let _ = actions.send(Action::Stop);
                    }
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::Worker(Update::ReadyToQuit | Update::ServiceRemoved)) => {
                quit = true
            }
            Event::UserEvent(UserEvent::Worker(update)) => view.update(update),
            _ => {}
        }
        if quit {
            view.tray.take();
            *control_flow = ControlFlow::Exit;
        }
    })
}
