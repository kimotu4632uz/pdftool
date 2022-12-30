use anyhow::Context;

use argorder;
use clap::{ArgAction, Parser};

use std::path::PathBuf;

use pdftool::Pdf;

/// CLI app to manipulate URLs and images in PDF
#[derive(Parser)]
#[clap(author, about, version)]
struct Arg {
    /// Set input file to INPUT. if not defined, make new PDF document.
    #[clap(short, long)]
    input: Option<PathBuf>,

    /// Set output file to OUTPUT. if not defined, overwrite input file.
    #[clap(short, long)]
    output: Option<PathBuf>,

    /// Set PDF author as AUTHOR
    #[clap(short, long)]
    author: Option<String>,

    /// Add LINK to PAGE
    #[clap(short = 'l', long, num_args = 2, value_names = ["LINK", "PAGE"])]
    add_link: Vec<String>,

    /// Add FILE to pdf
    #[clap(short = 'p', long, num_args = 0.. , value_name = "FILE")]
    add_page: Vec<String>,

    /// Remove link of PAGE
    #[clap(short = 'L', long, num_args = 0.. , value_name = "PAGE")]
    remove_link: Vec<u32>,

    /// Remove PAGE
    #[clap(short = 'P', long, num_args = 0.. , value_name = "PAGE")]
    remove_page: Vec<u32>,

    /// Move link from FROM to TO
    #[clap(short = 'm', long, num_args = 2, value_names = ["FROM", "TO"])]
    move_link: Vec<u32>,

    /// Move page from FROM to TO
    #[clap(short = 'M', long, num_args = 2, value_names = ["FROM", "TO"])]
    move_page: Vec<u32>,

    /// Prune unused object and renumber
    #[clap(short = 'c', long, action = ArgAction::Count)]
    prune: u8,
}

trait IterNextN: Iterator {
    fn nextn(&mut self, count: u32) -> Vec<Self::Item> {
        let mut result = Vec::with_capacity(count as usize);

        for _ in 0..count {
            let Some(val) = self.next() else {
                break;
            };
            result.push(val);
        }

        result
    }
}

impl<T: ?Sized> IterNextN for T where T: Iterator {}

fn main() -> anyhow::Result<()> {
    let (args, order) = argorder::parse::<Arg>();

    // check if input or output is avail
    anyhow::ensure!(
        args.input.is_some() || args.output.is_some(),
        "both input and output file not provided"
    );

    let mut pdf = if let Some(file) = &args.input {
        Pdf::load(file)?
    } else {
        Pdf::new()
    };

    let output = args.input.or(args.output).unwrap();

    let mut ali = args.add_link.into_iter();
    let mut api = args.add_page.into_iter();
    let mut rli = args.remove_link.into_iter();
    let mut rpi = args.remove_page.into_iter();
    let mut mli = args.move_link.into_iter();
    let mut mpi = args.move_page.into_iter();

    for (op, argc) in order {
        let op = op.as_str();

        match op {
            "author" => {
                pdf.set_author(args.author.as_ref().unwrap())?;
            }
            "add_link" => {
                let link = ali.next().unwrap();
                let page_str = ali.next().unwrap();
                let page: u32 = page_str.parse().with_context(|| {
                    format!("Invalid argument {} found in option \"{}\"", page_str, op)
                })?;

                pdf.add_link(&link, page)?;
            }
            "add_page" => {
                for file in api.nextn(argc) {
                    let bytes = std::fs::read(file)?;
                    let _ = pdf.add_image(&bytes)?;
                }
            }
            "remove_link" => {
                for page in rli.nextn(argc) {
                    pdf.remove_link(page)?;
                }
            }
            "remove_page" => {
                pdf.remove_pages(&rpi.nextn(argc));
            }
            "move_link" => {
                let from = mli.next().unwrap();
                let to = mli.next().unwrap();

                pdf.move_link(from, to)?;
            }
            "move_page" => {
                let from: usize = mpi.next().unwrap().try_into()?;
                let to: usize = mpi.next().unwrap().try_into()?;

                pdf.move_page(from, to)?;
            }
            "prune" => {
                pdf.prune();
            }
            _ => {}
        }
    }

    pdf.save(output)?;

    Ok(())
}
