use clap::clap_app;
use itertools::Itertools;
use lopdf::{Object, ObjectId, Document};

use pdftool::img2pdf::Pdf;

fn main() -> anyhow::Result<()> {
    let mut app = clap_app!(pdftool =>
        (@arg input: -i --input [FILE] "Set input file to FILE. if not defined, make new PDF document.")
        (@arg output: -o --output [FILE] "Set output file to FILE. if not defined, overwrite input file.")
        (@arg author: -a --("set-author") [STRING] "Set PDF author as STRING")
        (@arg alink: -l --("add-link") [LINK] [PAGE] "Add LINK to PAGE")
        (@arg aimage: -p --("add-image") [FILE]... "Add FILE to pdf")
        (@arg rlink: -L --("remove-link") [PAGE]... "Remove link of PAGE")
        (@arg rimg: -I --("remove-page") [PAGE]... "Remove PAGE")
        (@arg mlink: -m --("move-link") [FROM] [TO] "Move link from FROM to TO")
        (@arg mpage: -M --("move-page") [FROM] [TO] "Move page from FROM to TO")
        (@arg prune: -P --prune "Prune unused object and renumber")
    );

    if std::env::args().len() == 1 {
        app.print_help()?;
        return Ok(())
    }
    
    let matches = app.get_matches();

    let mut pdf: Pdf = if let Some(input) = matches.value_of("input") {
        Document::load(input)?.into()
    } else {
        Pdf::new()
    };

    let output = matches.value_of("output").or(matches.value_of("input")).expect("Error: both input and output file not provided");

    let pages = pdf.pdf.get_pages();

    if let Some(author) = matches.value_of("author") {
        pdf.set_author(author)?;
    } else if let Some(vec) = matches.values_of("alink") {
        let vec = vec.collect_vec();
        let link_str = vec[0];
        let page_num = vec[1].parse::<u32>().expect("Error: given page number isn't number");
        let page_id = *pages.get(&page_num).expect("Error: invalid page number");
        pdf.add_link(link_str, page_id)?;
    } else if let Some(vec) = matches.values_of("aimage") {
        for img in vec {
            let bytes = std::fs::read(img)?;
            let _ = pdf.add_image(&bytes)?;
        }
    } else if let Some(vec) = matches.values_of("rlink") {
        for page in vec {
            let page_id = *pages.get(&page.parse().unwrap()).expect("Error: invalid page number");
            pdf.remove_link(page_id)?;
        }
    } else if let Some(vec) = matches.values_of("rimg") {
        let vec = vec.map(|x| x.parse().unwrap()).collect_vec();
        pdf.pdf.delete_pages(&vec);
    } else if let Some(vec) = matches.values_of("mlink") {
        let vec = vec.collect_vec();
        let from = vec[0].parse().unwrap();
        let to = vec[1].parse().unwrap();
        let from_id = *pages.get(&from).expect("Error: invalid page number");
        let to_id = *pages.get(&to).expect("Error: invalid page number");
        pdf.move_link(from_id, to_id)?;
    } else if let Some(vec) = matches.values_of("mpage") {
        let vec = vec.collect_vec();
        let from = vec[0].parse().unwrap();
        let to = vec[1].parse().unwrap();

        let from_id = *pages.get(&from).expect("Error: invalid page number");
        let to_id = *pages.get(&to).expect("Error: invalid page number");

        let pages = pdf.pdf.get_object(pdf.pages_id).and_then(Object::as_dict).unwrap().get(b"Kids").and_then(Object::as_array).unwrap();

        if let Some((from_pages_id, from_pos)) = find_page_in_pages(&pdf.pdf, pages, pdf.pages_id, from_id) {
            if let Some((to_pages_id, to_pos)) = find_page_in_pages(&pdf.pdf, pages, pdf.pages_id, to_id) {
                pdf.pdf.get_object_mut(from_pages_id).and_then(Object::as_dict_mut).unwrap().get_mut(b"Kids").and_then(Object::as_array_mut).unwrap().remove(from_pos);
                pdf.pdf.get_object_mut(to_pages_id).and_then(Object::as_dict_mut).unwrap().get_mut(b"Kids").and_then(Object::as_array_mut).unwrap().insert(to_pos, from_id.into());
            } else {
                anyhow::bail!("to page is not found")
            }
        } else {
            anyhow::bail!("from page is not found")
        }
    } else if matches.is_present("prune") {
        let _ = pdf.pdf.prune_objects();
        let _ = pdf.pdf.renumber_objects();
    }

    pdf.pdf.save(output)?;

    Ok(())
}

fn find_page_in_pages(pdf: &Document, pages: &Vec<Object>, pages_id: ObjectId, target: ObjectId) -> Option<(ObjectId, usize)> {
    let mut target_pos = None;
    let mut pages_next = Vec::new();

    for (idx, page) in pages.iter().enumerate() {
        let page_id = page.as_reference().unwrap();

        if page_id == target {
            target_pos = Some((pages_id, idx));
        } else {
            let obj = pdf.get_object(page_id).unwrap();
            if let Object::Dictionary(_) = obj {
                if obj.type_name().unwrap() == "Pages" {
                    pages_next.push(page_id);
                }
            }
        }
    }

    if target_pos.is_some() {
        target_pos
    } else if pages_next.is_empty() {
        None
    } else {
        for pages_id in pages_next {
            let pages_vec = pdf.get_object(pages_id).and_then(Object::as_dict).unwrap().get(b"Kids").and_then(Object::as_array).unwrap();
            let result = find_page_in_pages(pdf, pages_vec, pages_id, target);
            if result.is_some() {
                return result;
            }
        }
        None
    }
}
