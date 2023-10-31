use base64::engine::general_purpose;
use base64::Engine as _;
use notify::RecursiveMode;
use pdfium_render::prelude::*;

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;
use std::{env};

use anyhow::{bail, Result};
use notify_debouncer_mini::new_debouncer;
use std::io::{stdin, Write};
use std::io::{stdout, Cursor};
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;

#[derive(Debug)]
struct Pdf {
    file: String,
    page: Page,
    current_page: usize,
    length: usize,
    text: Vec<String>,
}

#[derive(Debug)]
struct FileList {
    files: Vec<String>,
    current_file: usize,
}

impl FileList {
    fn current(&self) -> String {
        let current = self.current_file;
        self.files[current].clone()
    }

    fn next(&mut self) {
        if self.current_file != (self.files.len() - 1) {
            self.current_file = self.current_file + 1;
        }
    }

    fn prev(&mut self) {
        if self.current_file != 0 {
            self.current_file = self.current_file - 1;
        }
    }
}

#[derive(Clone, Debug)]
struct Page {
    data: Vec<u8>,
    size: (u32, u32),
}

#[derive(Debug)]
enum Msg {
    NextPage,
    PreviousPage,
    NextDocument,
    PreviousDocument,
    Refresh,
    Quit,
    Open,
    // Rotate,
    None,
    LastPage,
    FirstPage,
}

impl From<Key> for Msg {
    fn from(item: Key) -> Self {
        match item {
            Key::Char('j') => Msg::NextPage,
            Key::Down => Msg::NextPage,
            Key::Char('k') => Msg::PreviousPage,
            Key::Up => Msg::PreviousPage,
            Key::Char('r') => Msg::Refresh,
            Key::Char('q') => Msg::Quit,
            Key::Char('o') => Msg::Open,
            Key::Char('l') => Msg::NextDocument,
            Key::Char('h') => Msg::PreviousDocument,
            Key::Left => Msg::PreviousDocument,
            Key::Right => Msg::NextDocument,
            Key::Char('G') => Msg::LastPage,
            Key::Char('g') => Msg::FirstPage,
            // Key::Char('w') => Msg::Rotate,
            _ => Msg::None,
        }
    }
}

impl Page {
    fn display(&self, r: Option<bool>) -> Result<()> {
        let size = termion::terminal_size();

        let (cols, rows) = match size {
            Ok((c, r)) => (c, r),
            _ => anyhow::bail!("Whoops"),
        };

        let mut stdout = stdout();

        let mut pdf_aspect_ratio = (self.size.0 as i32 / self.size.1 as i32) >= 1;
        let mut term_aspect_ratio = (cols as i32 / rows as i32) >= 1;

        if r.is_some()  {
            pdf_aspect_ratio = !pdf_aspect_ratio;
            term_aspect_ratio = !term_aspect_ratio;
        } 
        if (pdf_aspect_ratio == false) & (term_aspect_ratio == true) {
            write!(stdout, "{}", termion::cursor::Goto(1, 1))?;
            writeln!(
                stdout,
                "\x1b]1337;File=inline=1;preserveAspectRatio=1;size={};height={}:{}\x07",
                self.data.len(),
                rows - 2,
                general_purpose::STANDARD.encode(&self.data)
            )?;
        } else {
            write!(stdout, "{}", termion::cursor::Goto(1, 1))?;
            writeln!(
                stdout,
                "\x1b]1337;File=inline=1;preserveAspectRatio=1;size={};width={}:{}\x07",
                self.data.len(),
                cols - 2,
                general_purpose::STANDARD.encode(&self.data)
            )?;
        }
        Ok(())
    }
}

pub trait Apply<Res> {
    fn apply<F: FnOnce(Self) -> Res>(self, f: F) -> Res
    where
        Self: Sized,
    {
        f(self)
    }

    fn apply_ref<F: FnOnce(&Self) -> Res>(&self, f: F) -> Res {
        f(self)
    }

    fn apply_mut<F: FnOnce(&mut Self) -> Res>(&mut self, f: F) -> Res {
        f(self)
    }
}

impl<T: ?Sized, Res> Apply<Res> for T {}

impl Pdf {
    fn get_page(&mut self, p: usize) {
        let pdfium = Pdfium::new(
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(
                "/usr/local/lib/",
            ))
            .unwrap(),
        );

        let document = pdfium.load_pdf_from_file(&self.file, None).unwrap();

        let render_config = PdfRenderConfig::new()
            .set_target_height(1920)
            .use_lcd_text_rendering(false)
            .disable_native_text_rendering(false)
            .rotate_if_landscape(PdfBitmapRotation::Degrees90, true);

        let page: Page = document
            .pages()
            .get(p as u16)
            // .iter()
            .apply(|page| {
                let mut height: u32 = 0;
                let mut width: u32 = 0;
                let mut buffer: Cursor<Vec<u8>> = std::io::Cursor::new(vec![]);
                page.unwrap()
                    .render_with_config(&render_config)
                    .expect("Error")
                    .as_image()
                    .apply(|x| {
                        height = x.height();
                        width = x.width();
                        x
                    })
                    .write_to(&mut buffer, image::ImageFormat::Tiff)
                    .expect("Error");
                let p = Page {
                    data: buffer.into_inner(),
                    size: (width, height),
                };
                return p;
            });
        // .collect();

        self.page = page;
        self.current_page = p;
    }

    fn new(file: &String, current_page: Option<usize>) -> Result<Pdf> {
        let p = match current_page {
            None => 0,
            Some(v) => v,
        };
        let pdfium = Pdfium::new(Pdfium::bind_to_library(
            Pdfium::pdfium_platform_library_name_at_path("/usr/local/lib/"),
        )?);

        let document = pdfium.load_pdf_from_file(&file, None)?;

        let render_config = PdfRenderConfig::new()
            .set_target_height(1920)
            .use_lcd_text_rendering(false)
            .disable_native_text_rendering(false)
            .rotate_if_landscape(PdfBitmapRotation::Degrees90, true);

        let length = document.pages().len() as usize;

        let page: Page = document
            .pages()
            .get(p as u16)
            // .iter()
            .apply(|page| {
                let mut height: u32 = 0;
                let mut width: u32 = 0;
                let mut buffer: Cursor<Vec<u8>> = std::io::Cursor::new(vec![]);
                page.unwrap()
                    .render_with_config(&render_config)
                    .expect("Error")
                    .as_image()
                    .apply(|x| {
                        height = x.height();
                        width = x.width();
                        x
                    })
                    .write_to(&mut buffer, image::ImageFormat::Tiff)
                    .expect("Error");
                let p = Page {
                    data: buffer.into_inner(),
                    size: (width, height),
                };
                return p;
            });
        // .collect();

        /*
        let text = document
            .pages()
            .iter()
            .map(|page| page.text().expect("Error reading text").to_string())
            .collect(); */

        let text = vec![];

        Ok(Pdf {
            file: file.clone(),
            page,
            current_page: p,
            length,
            text,
        })
    }
}

fn main() {
    let files: Vec<String> = env::args().skip(1).map(|x| x).collect();

    let file = match files.len() {
        0 => None,
        _ => Some(files),
    };
    let files = match file {
        Some(f) => f,
        None => glob::glob("./*.pdf")
            .unwrap()
            .map(|x| {
                let item = match x {
                    Ok(v) => v.to_str().expect("Error with file").to_string(),
                    Err(_) => {
                        eprintln!("Couldn't find pdf files");
                        std::process::exit(1);
                    }
                };
                item
            })
            .collect(),
    };

    if files.len() == 0 {
        eprintln!("Couldn't find pdf files");
        std::process::exit(1);
    };

    let files = FileList {
        files,
        current_file: 0,
    };
    let res = runmulti(files);
    match res {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            eprintln!("I encountered an erorr! {}", e.to_string());
            std::process::exit(1);
        }
    };
}

fn runmulti(mut files: FileList) -> anyhow::Result<()> {
    let file = files.current();
    let file2 = file.clone();

    let mut pdf = match Pdf::new(&file.clone(), None) {
        Ok(v) => v,
        Err(_) => bail!("Couldn't load pdf or not a valid pdf file"),
    };

    let (tx, rx) = mpsc::channel();
    let tx1 = tx.clone();
    thread::spawn(move || {
        let p = Path::new(&file2);
        let (tx2, rx2) = std::sync::mpsc::channel();
        let mut watcher = match new_debouncer(Duration::from_secs(2), None, tx2) {
            Ok(v) => v,
            Err(e) => bail!("{:?}", e.kind),
        };
        watcher
            .watcher()
            .watch(p.as_ref(), RecursiveMode::Recursive)
            .expect("Couldn't create file watcher");

        for res in rx2 {
            match res {
                Ok(_) => tx1
                    .send(Msg::Refresh)
                    .expect("Couldn't send REFRESH command"),
                _ => {}
            }
        }
        Ok(())
    });
    thread::spawn(move || {
        let stdin = stdin();
        for c in stdin.keys() {
            match c {
                key => match key {
                    Ok(v) => tx.send(v.into()).expect("Couldn't send key press"),
                    _ => {}
                },
            };
        }
    });
    loop {
        let res = browser(&mut pdf, &rx); //, &refresh);
        match res.expect("Error in browser") {
            Refersh::Done => {
                println!("");
                println!("{}", pdf.file);
                return Ok(());
            }
            Refersh::Oker => {
                let p = pdf.current_page;
                pdf = Pdf::new(&file.clone().to_owned(), Some(p)).expect("Couldn't refresh file");
            }
            Refersh::Next => {
                files.next();
                let file = files.current();
                pdf = Pdf::new(&file.clone().to_owned(), None).expect("Couldn't refresh file");
            }
            Refersh::Previous => {
                files.prev();
                let file = files.current();
                pdf = Pdf::new(&file.clone().to_owned(), None).expect("Couldn't refresh file");
            }
        }
    }
    // Ok(())
}

fn run(file: String) -> anyhow::Result<()> {
    let file2 = file.clone();
    let mut pdf = match Pdf::new(&file.clone(), None) {
        Ok(v) => v,
        Err(_) => bail!("Couldn't load pdf or not a valid pdf file"),
    };

    let (tx, rx) = mpsc::channel();
    let tx1 = tx.clone();
    thread::spawn(move || {
        let p = Path::new(&file2);
        let (tx2, rx2) = std::sync::mpsc::channel();
        let mut watcher = match new_debouncer(Duration::from_secs(2), None, tx2) {
            Ok(v) => v,
            Err(e) => bail!("{:?}", e.kind),
        };
        watcher
            .watcher()
            .watch(p.as_ref(), RecursiveMode::Recursive)
            .expect("Couldn't create file watcher");

        for res in rx2 {
            match res {
                Ok(_) => tx1
                    .send(Msg::Refresh)
                    .expect("Couldn't send REFRESH command"),
                _ => {}
            }
        }
        Ok(())
    });
    thread::spawn(move || {
        let stdin = stdin();
        for c in stdin.keys() {
            match c {
                key => match key {
                    Ok(v) => tx.send(v.into()).expect("Couldn't send key press"),
                    _ => {}
                },
            };
        }
    });
    loop {
        let res = browser(&mut pdf, &rx); //, &refresh);
        match res.expect("Error in browser") {
            Refersh::Done => {
                println!("{}", pdf.file);
                return Ok(());
            }
            Refersh::Oker => {
                let p = pdf.current_page;
                pdf = Pdf::new(&file.clone().to_owned(), Some(p)).expect("Couldn't refresh file");
            }
            _ => {}
        }
    }
    // Ok(())
}

enum Refersh {
    Oker,
    Done,
    Next,
    Previous,
}

fn browser(pdf: &mut Pdf, rx: &Receiver<Msg>) -> anyhow::Result<Refersh> {
    let mut stdout = stdout().into_raw_mode()?;

    write!(
        stdout,
        "{}{}",
        termion::cursor::Restore,
        termion::clear::CurrentLine
    )?;
    write!(
        stdout,
        "{}{}",
        termion::cursor::Goto(1, 1),
        termion::clear::All,
    )?;

    pdf.page.display(None)?;

    let mut double_gg = false;
    for c in rx {
        match c {
            Msg::FirstPage => match double_gg {
                true => {
                    pdf.current_page = 0;
                    pdf.get_page(pdf.current_page);
                    pdf.page.display(None)?;
                }
                false => {
                    double_gg = true;
                }
            },
            Msg::LastPage => {
                pdf.current_page = pdf.length - 1;
                pdf.get_page(pdf.current_page);
                pdf.page.display(None)?;
            }
            Msg::None => {}
            Msg::Quit => return Ok(Refersh::Done),
            Msg::Open => {
                Command::new("open")
                    .arg(&pdf.file)
                    .spawn()
                    .expect("Couldn't open file in external application");
            }
            Msg::Refresh => return Ok(Refersh::Oker),
            Msg::NextPage => {
                double_gg = false;
                if pdf.current_page != (pdf.length - 1) {
                    pdf.current_page = pdf.current_page + 1;
                    pdf.get_page(pdf.current_page);
                    pdf.page.display(None)?;
                };
            }
            Msg::PreviousPage => {
                double_gg = false;
                if pdf.current_page != 0 {
                    pdf.current_page = pdf.current_page - 1;
                    pdf.get_page(pdf.current_page);
                    pdf.page.display(None)?;
                }
            },
            /* Msg::Rotate => {
                pdf.page.display(Some(true))?;
            }, */

            Msg::NextDocument => return Ok(Refersh::Next),
            Msg::PreviousDocument => return Ok(Refersh::Previous),
        }
    }

    Ok(Refersh::Done)
}
