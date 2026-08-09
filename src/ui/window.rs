//! The window: one navigation stack, three pages deep.
//!
//! Search, then which pressing, then where to buy it. A drill-down rather than
//! tabs or a sidebar, because the three are strictly sequential and each one's
//! content is chosen by the last — which is exactly what `AdwNavigationView` is
//! for, and it gets the back gesture and the breadcrumb title for free.

use std::cell::OnceCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::editions_page::EditionsPage;
use super::result_page::ResultPage;
use super::search_page::SearchPage;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SleeveWindow {
        pub navigation: OnceCell<adw::NavigationView>,
        pub search: OnceCell<Rc<SearchPage>>,
        pub editions: OnceCell<Rc<EditionsPage>>,
        pub result: OnceCell<Rc<ResultPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SleeveWindow {
        const NAME: &'static str = "SleeveWindow";
        type Type = super::SleeveWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for SleeveWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();

            let search = SearchPage::new();
            let editions = EditionsPage::new();
            let result = ResultPage::new();

            let navigation = adw::NavigationView::new();
            navigation.add(&search.page);

            window.set_content(Some(&navigation));
            window.set_default_size(760, 700);
            window.set_title(Some("Sleeve"));

            let _ = self.navigation.set(navigation);
            let _ = self.search.set(search);
            let _ = self.editions.set(editions);
            let _ = self.result.set(result);
        }
    }

    impl WidgetImpl for SleeveWindow {}
    impl WindowImpl for SleeveWindow {}
    impl ApplicationWindowImpl for SleeveWindow {}
    impl AdwApplicationWindowImpl for SleeveWindow {}
}

glib::wrapper! {
    pub struct SleeveWindow(ObjectSubclass<imp::SleeveWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
                    gtk::Root, gtk::ShortcutManager, gio::ActionGroup, gio::ActionMap;
}

impl SleeveWindow {
    pub fn search(&self) -> Rc<SearchPage> {
        Rc::clone(self.imp().search.get().expect("a search page"))
    }

    pub fn editions(&self) -> Rc<EditionsPage> {
        Rc::clone(self.imp().editions.get().expect("an editions page"))
    }

    pub fn result(&self) -> Rc<ResultPage> {
        Rc::clone(self.imp().result.get().expect("a result page"))
    }

    fn navigation(&self) -> adw::NavigationView {
        self.imp()
            .navigation
            .get()
            .expect("a navigation view")
            .clone()
    }

    /// Show the editions page, pushing it if it is not already up.
    pub fn show_editions(&self) {
        self.push("editions", &self.editions().page);
    }

    pub fn show_result(&self) {
        self.push("result", &self.result().page);
    }

    /// Push a page unless it is already the visible one.
    ///
    /// Choosing a second candidate while the editions page is up should replace
    /// its contents, not stack a second copy behind it — the back button would
    /// then walk through pages the person never visited.
    fn push(&self, tag: &str, page: &adw::NavigationPage) {
        let navigation = self.navigation();
        if navigation
            .visible_page()
            .and_then(|visible| visible.tag())
            .is_some_and(|visible| visible == tag)
        {
            return;
        }
        navigation.push(page);
    }
}

use gtk::gio;
