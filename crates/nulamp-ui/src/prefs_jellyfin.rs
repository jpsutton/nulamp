//! The "Jellyfin" page of the preferences dialog: signing in to a server and choosing which
//! of its music libraries the media library shows. Server requests run on a helper thread;
//! the dialog polls for the answer.

use std::sync::mpsc::{self, Receiver};

use nulamp_library::jellyfin::{self, Account, Library};
use nulamp_skinui::{Form, FormEvent, Rect, Widget};

const SERVER: u32 = 400;
const USER: u32 = 401;
const PASSWORD: u32 = 402;
pub(crate) const CONNECT: u32 = 403;
const LIBRARIES: u32 = 404;
const SHOW: u32 = 405;
const REFRESH: u32 = 406;

/// A server answer.
enum Answer {
    Login(Result<Account, String>),
    /// The libraries of the account with this token.
    Libraries(String, Result<Vec<Library>, String>),
}

pub struct JellyfinPage {
    server: String,
    user: String,
    password: String,
    status: String,
    /// Selected row of the library list.
    selected: Option<usize>,
    pending: Option<Receiver<Answer>>,
    /// The library list was fetched since the dialog opened.
    refreshed: bool,
    /// Server calls (replaced in tests).
    pub(crate) login: fn(&str, &str, &str) -> nulamp_library::Result<Account>,
    pub(crate) libraries: fn(&Account) -> nulamp_library::Result<Vec<Library>>,
}

/// Indexes (into `account.libraries`) of the libraries the page lists: music only for now.
fn listed(account: &Account) -> Vec<usize> {
    (0..account.libraries.len())
        .filter(|&i| account.libraries[i].is_music())
        .collect()
}

fn signed_in(a: &Account) -> String {
    format!("Signed in to {} as {}.", a.server_name, a.user_name)
}

impl JellyfinPage {
    pub fn new(account: Option<&Account>) -> Self {
        Self {
            server: account.map(|a| a.server_url.clone()).unwrap_or_default(),
            user: account.map(|a| a.user_name.clone()).unwrap_or_default(),
            password: String::new(),
            status: account.map_or_else(|| "Not signed in.".to_string(), signed_in),
            selected: None,
            pending: None,
            refreshed: false,
            login: jellyfin::login,
            libraries: jellyfin::libraries,
        }
    }

    /// The page's controls in the area from (`x`, `y`), `w` wide, down to `bottom`.
    pub fn widgets(&self, account: Option<&Account>, x: i32, w: i32, bottom: i32) -> Vec<Widget> {
        let mut out = Vec::new();
        let mut y = 6;
        for (label, id, text, secret) in [
            ("Server:", SERVER, &self.server, false),
            ("User name:", USER, &self.user, false),
            ("Password:", PASSWORD, &self.password, true),
        ] {
            out.push(Widget::Label {
                rect: Rect::new(x, y + 2, 70, 14),
                text: label.into(),
            });
            out.push(Widget::Edit {
                rect: Rect::new(x + 74, y, w - 74, 18),
                id,
                text: text.clone(),
                cursor: 0,
                secret,
            });
            y += 24;
        }
        out.push(Widget::Button {
            rect: Rect::new(x, y + 2, 70, 18),
            id: CONNECT,
            label: if account.is_some() {
                "Sign out"
            } else {
                "Sign in"
            }
            .into(),
            default: false,
        });
        out.push(Widget::Label {
            rect: Rect::new(x + 76, y + 4, w - 76, 14),
            text: self.status.clone(),
        });
        y += 30;
        out.push(Widget::Label {
            rect: Rect::new(x, y, w, 14),
            text: "Music libraries (double-click to show or hide):".into(),
        });
        let rows: Vec<String> = account
            .map(|a| {
                listed(a)
                    .into_iter()
                    .map(|i| {
                        let l = &a.libraries[i];
                        format!("[{}] {}", if l.enabled { "x" } else { " " }, l.name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let list_h = (bottom - (y + 18) - 26).max(30);
        let selected = self.selected.filter(|&s| s < rows.len());
        let shown = account
            .zip(selected)
            .is_some_and(|(a, s)| a.libraries[listed(a)[s]].enabled);
        out.push(Widget::List {
            rect: Rect::new(x, y + 18, w, list_h),
            id: LIBRARIES,
            rows,
            selected,
            first: 0,
        });
        let by = y + 18 + list_h + 6;
        out.push(Widget::Check {
            rect: Rect::new(x, by + 1, w - 80, 16),
            id: SHOW,
            label: "Show in the media library".into(),
            on: shown,
        });
        out.push(Widget::Button {
            rect: Rect::new(x + w - 70, by, 70, 18),
            id: REFRESH,
            label: "Refresh".into(),
            default: false,
        });
        out
    }

    /// Greys out what cannot be used now, and starts the first library refresh.
    pub fn after_build(&mut self, form: &mut Form, account: Option<&Account>) {
        let busy = self.pending.is_some();
        for id in [SERVER, USER, PASSWORD] {
            form.set_enabled(id, account.is_none() && !busy);
        }
        form.set_enabled(CONNECT, !busy);
        form.set_enabled(REFRESH, account.is_some() && !busy);
        let selected = self.selected.is_some() && account.is_some_and(|a| !listed(a).is_empty());
        form.set_enabled(SHOW, selected);
        if let Some(a) = account
            && !self.refreshed
            && !busy
        {
            self.refresh(a);
        }
    }

    /// Copies the edit fields back.
    pub fn collect(&mut self, form: &Form) {
        if form.find(SERVER).is_some() {
            self.server = form.text(SERVER);
            self.user = form.text(USER);
            self.password = form.text(PASSWORD);
        }
    }

    fn refresh(&mut self, account: &Account) {
        let (tx, rx) = mpsc::channel();
        let (a, libraries) = (account.clone(), self.libraries);
        std::thread::spawn(move || {
            let r = libraries(&a).map_err(|e| e.to_string());
            let _ = tx.send(Answer::Libraries(a.token, r));
        });
        self.pending = Some(rx);
        self.refreshed = true;
    }

    /// Whether Enter should sign in (signed out, nothing running) rather than press OK.
    pub fn signs_in_on_enter(&self, account: Option<&Account>) -> bool {
        account.is_none() && self.pending.is_none()
    }

    /// Handles a page event; `false` for events of other controls. The caller collects the
    /// fields first and rebuilds the form afterwards.
    pub fn event(&mut self, ev: &FormEvent, account: &mut Option<Account>) -> bool {
        match ev {
            FormEvent::Clicked(CONNECT) if self.pending.is_none() => {
                if account.take().is_some() {
                    // The session ends on the server when the preferences are applied.
                    self.status = "Not signed in.".into();
                    self.selected = None;
                } else {
                    let (tx, rx) = mpsc::channel();
                    let (server, user, password, login) = (
                        self.server.clone(),
                        self.user.clone(),
                        self.password.clone(),
                        self.login,
                    );
                    std::thread::spawn(move || {
                        let r = login(&server, &user, &password).map_err(|e| e.to_string());
                        let _ = tx.send(Answer::Login(r));
                    });
                    self.pending = Some(rx);
                    self.status = "Signing in…".into();
                }
            }
            FormEvent::Clicked(REFRESH) if self.pending.is_none() => {
                if let Some(a) = account {
                    self.refresh(a);
                    self.status = "Refreshing…".into();
                }
            }
            FormEvent::Selected(LIBRARIES, i) => self.selected = Some(*i),
            FormEvent::Activated(LIBRARIES, i) => {
                self.selected = Some(*i);
                self.toggle(account);
            }
            FormEvent::Clicked(SHOW) => self.toggle(account),
            _ => return false,
        }
        true
    }

    /// Shows or hides the selected library (the check box mirrors its state).
    fn toggle(&mut self, account: &mut Option<Account>) {
        let Some(a) = account.as_mut() else {
            return;
        };
        if let Some(i) = self.selected.and_then(|s| listed(a).get(s).copied()) {
            a.libraries[i].enabled = !a.libraries[i].enabled;
        }
    }

    /// Takes a server answer. Returns whether the page changed.
    pub fn poll(&mut self, account: &mut Option<Account>) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let answer = match rx.try_recv() {
            Ok(a) => a,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                return true;
            }
        };
        self.pending = None;
        match answer {
            Answer::Login(Ok(a)) => {
                self.status = signed_in(&a);
                self.server = a.server_url.clone();
                self.password.clear();
                self.selected = None;
                self.refreshed = true;
                *account = Some(a);
            }
            Answer::Login(Err(e)) => self.status = format!("Cannot sign in: {e}"),
            Answer::Libraries(token, r) => match (account.as_mut(), r) {
                (Some(a), Ok(mut libraries)) if a.token == token => {
                    // Choices made while the list was loading win.
                    for l in &mut libraries {
                        if let Some(old) = a.libraries.iter().find(|o| o.id == l.id) {
                            l.enabled = old.enabled;
                        }
                    }
                    a.libraries = libraries;
                    self.status = signed_in(a);
                }
                (Some(a), Err(e)) if a.token == token => {
                    self.status = format!("Cannot list the libraries: {e}");
                }
                _ => {}
            },
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(id: &str, kind: &str, enabled: bool) -> Library {
        Library {
            id: id.into(),
            name: id.to_uppercase(),
            collection_type: kind.into(),
            enabled,
        }
    }

    fn fake_login(server: &str, user: &str, password: &str) -> nulamp_library::Result<Account> {
        if password != "pw" {
            return Err(nulamp_library::Error::Provider(
                "wrong user name or password".into(),
            ));
        }
        Ok(Account {
            server_url: format!("http://{server}"),
            server_name: "Home".into(),
            user_name: user.into(),
            user_id: "u".into(),
            token: "t1".into(),
            libraries: vec![
                lib("a", "music", true),
                lib("m", "movies", false),
                lib("b", "music", true),
            ],
            ..Account::default()
        })
    }

    fn fake_libraries(a: &Account) -> nulamp_library::Result<Vec<Library>> {
        let mut l = a.libraries.clone();
        l.push(lib("c", "music", true));
        Ok(l)
    }

    fn wait(p: &mut JellyfinPage, account: &mut Option<Account>) {
        for _ in 0..500 {
            if p.poll(account) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("no answer");
    }

    fn page() -> JellyfinPage {
        let mut p = JellyfinPage::new(None);
        p.login = fake_login;
        p.libraries = fake_libraries;
        p
    }

    #[test]
    fn sign_in_choose_and_sign_out() {
        let mut p = page();
        let mut account = None;
        p.server = "jf:8096".into();
        p.user = "me".into();
        p.password = "bad".into();
        assert!(p.event(&FormEvent::Clicked(CONNECT), &mut account));
        assert_eq!(p.status, "Signing in…");
        wait(&mut p, &mut account);
        assert!(account.is_none());
        assert!(p.status.contains("wrong user name"), "{}", p.status);

        p.password = "pw".into();
        p.event(&FormEvent::Clicked(CONNECT), &mut account);
        wait(&mut p, &mut account);
        let a = account.as_ref().unwrap();
        assert_eq!(a.user_name, "me");
        assert_eq!(p.status, "Signed in to Home as me.");
        assert!(p.password.is_empty(), "the password is not kept");
        assert_eq!(p.server, "http://jf:8096");

        // Only music libraries are listed; the second row is library `b`.
        let w = p.widgets(account.as_ref(), 134, 316, 280);
        let rows = w
            .iter()
            .find_map(|w| match w {
                Widget::List {
                    id: LIBRARIES,
                    rows,
                    ..
                } => Some(rows.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(rows, ["[x] A", "[x] B"]);
        p.event(&FormEvent::Activated(LIBRARIES, 1), &mut account);
        assert!(!account.as_ref().unwrap().libraries[2].enabled);
        p.event(&FormEvent::Clicked(SHOW), &mut account);
        assert!(account.as_ref().unwrap().libraries[2].enabled);

        // Refreshing adds new libraries and keeps the choices.
        account.as_mut().unwrap().libraries[0].enabled = false;
        p.event(&FormEvent::Clicked(REFRESH), &mut account);
        wait(&mut p, &mut account);
        let ids: Vec<_> = account
            .as_ref()
            .unwrap()
            .libraries
            .iter()
            .map(|l| (l.id.as_str(), l.enabled))
            .collect();
        assert_eq!(ids, [("a", false), ("m", false), ("b", true), ("c", true)]);

        p.event(&FormEvent::Clicked(CONNECT), &mut account);
        assert!(account.is_none());
        assert_eq!(p.status, "Not signed in.");
    }

    #[test]
    fn opening_signed_in_refreshes_once() {
        let a = fake_login("x", "me", "pw").unwrap();
        let mut p = JellyfinPage::new(Some(&a));
        p.libraries = fake_libraries;
        let mut account = Some(a);
        let mut form = Form::new(p.widgets(account.as_ref(), 134, 316, 280));
        p.after_build(&mut form, account.as_ref());
        assert!(
            !form.is_enabled(SERVER),
            "fields are locked while signed in"
        );
        wait(&mut p, &mut account);
        assert_eq!(account.as_ref().unwrap().libraries.len(), 4);
        p.after_build(&mut form, account.as_ref());
        assert!(p.pending.is_none(), "no second refresh");
    }
}
