use std::{io::Cursor, path::Path};

use anyhow::anyhow;
use chrono::offset::Utc;
use image::{DynamicImage, GenericImageView, ImageFormat};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

struct PagesDict<'a> {
    dict: &'a Dictionary,
}

impl<'a> PagesDict<'a> {
    fn new(doc: &'a Document, pages_id: ObjectId) -> Self {
        Self {
            dict: doc.get_dictionary(pages_id).unwrap(),
        }
    }

    fn kids(&self) -> &Vec<Object> {
        self.dict.get(b"Kids").unwrap().as_array().unwrap()
    }
}

struct PagesDictMut<'a> {
    dict: &'a mut Dictionary,
}

impl<'a> PagesDictMut<'a> {
    fn new(doc: &'a mut Document, pages_id: ObjectId) -> Self {
        Self {
            dict: doc.get_dictionary_mut(pages_id).unwrap(),
        }
    }

    fn count(&self) -> i64 {
        self.dict.get(b"Count").unwrap().as_i64().unwrap()
    }

    fn set_count(&mut self, count: i64) {
        self.dict.set("Count", count);
    }

    fn kids_mut(&mut self) -> &mut Vec<Object> {
        self.dict.get_mut(b"Kids").unwrap().as_array_mut().unwrap()
    }

    fn push(&mut self, page_id: ObjectId) {
        self.kids_mut().push(page_id.into());
        self.set_count(self.count() + 1);
    }

    fn insert(&mut self, index: usize, page_id: ObjectId) {
        self.kids_mut().insert(index, page_id.into());
        self.set_count(self.count() + 1);
    }

    fn remove(&mut self, index: usize) -> Option<ObjectId> {
        let result = self.kids_mut().remove(index).as_reference().unwrap();
        self.set_count(self.count() - 1);
        Some(result)
    }
}

struct Pages<'a> {
    doc: &'a mut Document,
    root_id: ObjectId,
}

impl<'a> Pages<'a> {
    fn new(doc: &'a mut Document, pages_id: ObjectId) -> Self {
        Pages {
            doc,
            root_id: pages_id,
        }
    }

    fn push(&mut self, page_id: ObjectId) {
        PagesDictMut::new(self.doc, self.root_id).push(page_id);
    }

    fn find_pages(&self, index: &mut usize, pages_id: ObjectId) -> Option<(ObjectId, usize)> {
        for (i, kid) in PagesDict::new(self.doc, pages_id)
            .kids()
            .iter()
            .map(|x| x.as_reference().unwrap())
            .enumerate()
        {
            match self.doc.get_dictionary(kid).unwrap().type_name().unwrap() {
                "Pages" => {
                    let result = self.find_pages(index, kid);
                    if result.is_some() {
                        return result;
                    }
                }
                "Page" => {
                    *index -= 1;
                    if *index == 0 {
                        return Some((pages_id, i));
                    }
                }
                _ => {}
            };
        }
        return None;
    }

    fn insert(&mut self, index: usize, page_id: ObjectId) {
        let mut index = index;
        if let Some((id, idx)) = self.find_pages(&mut index, self.root_id) {
            PagesDictMut::new(self.doc, id).insert(idx, page_id);
        }
    }

    fn remove(&mut self, index: usize) -> Option<ObjectId> {
        let mut index = index;
        self.find_pages(&mut index, self.root_id)
            .and_then(|(id, idx)| PagesDictMut::new(self.doc, id).remove(idx))
    }
}

pub struct Pdf {
    pub doc: Document,
    pub pages_id: ObjectId,
}

impl Pdf {
    pub fn new() -> Self {
        let mut doc = Document::with_version("1.7");

        let info_id = doc.add_object(dictionary! {
            "CreationDate" => Utc::now(),
            "ModDate" => Utc::now(),
        });

        doc.trailer.set("Info", info_id);

        let catalog_id = doc.new_object_id();
        doc.trailer.set("Root", catalog_id);

        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Count" => 0,
            "Kids" => vec![],
        });

        doc.objects.insert(
            catalog_id,
            dictionary! {
                "Type" => "Catalog",
                "Pages" => pages_id,
            }
            .into(),
        );

        Self { doc, pages_id }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let doc = Document::load(path)?;

        let pages_id = doc
            .catalog()?
            .get(b"Pages")
            .and_then(Object::as_reference)
            .unwrap()
            .to_owned();

        Ok(Self { doc, pages_id })
    }

    fn get_pages(&mut self) -> Pages {
        Pages::new(&mut self.doc, self.pages_id)
    }

    fn get_page_id(&self, num: u32) -> anyhow::Result<ObjectId> {
        self.doc
            .get_pages()
            .get(&num)
            .ok_or(anyhow!("Page {} not found in document.", num))
            .copied()
    }

    pub fn set_author(&mut self, author: &str) -> anyhow::Result<()> {
        let author_iter = author.encode_utf16();

        let mut utfbe_str: Vec<u8> = Vec::with_capacity((author_iter.count() + 1) * 2);
        utfbe_str.push(0xfe);
        utfbe_str.push(0xff);

        for byte in author.encode_utf16() {
            let u8_2 = byte.to_be_bytes();
            utfbe_str.push(u8_2[0]);
            utfbe_str.push(u8_2[1]);
        }

        let info = self
            .doc
            .trailer
            .get(b"Info")
            .and_then(Object::as_reference)?;

        self.doc
            .get_dictionary_mut(info)?
            .set(
                "Author",
                Object::String(utfbe_str, StringFormat::Hexadecimal),
            );

        Ok(())
    }

    pub fn add_link(&mut self, link: &str, page: u32) -> anyhow::Result<()> {
        let page_id = self.get_page_id(page)?;

        let rect = self
            .doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)?
            .get(b"MediaBox")?
            .to_owned();

        let url_id = self.doc.add_object(dictionary! {
            "S" => "URI",
            "URI" => Object::string_literal(link),
        });

        let annot_id = self.doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "A" => url_id,
            "Rect" => rect,
            "Border" => vec![0.into(), 0.into(), 0.into()],
            "F" => 4,
        });

        let page = self
            .doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)?;

        page.set("Annots", vec![annot_id.into()]);

        Ok(())
    }

    pub fn remove_link(&mut self, page: u32) -> anyhow::Result<()> {
        let page_id = self.get_page_id(page)?;

        self.doc
            .get_dictionary_mut(page_id)?
            .remove(b"Annots");

        Ok(())
    }

    pub fn move_link(&mut self, from: u32, to: u32) -> anyhow::Result<()> {
        let from_id = self.get_page_id(from)?;
        let to_id = self.get_page_id(to)?;

        let annots = self
            .doc
            .get_dictionary_mut(from_id)?
            .remove(b"Annots");

        if let Some(annots) = annots {
            self.doc
                .get_dictionary_mut(to_id)?
                .set("Annots", annots);
        }

        Ok(())
    }

    pub fn add_page(&mut self, width: u32, height: u32) -> anyhow::Result<ObjectId> {
        let page_id = self.doc.new_object_id();
        let contents_id = self.doc.add_object(Stream::new(dictionary! {}, vec![]));

        self.doc.objects.insert(
            page_id,
            dictionary! {
                "Type" => "Page",
                "Parent" => self.pages_id,
                "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
                "Contents" => contents_id,
            }
            .into(),
        );

        self.get_pages().push(page_id);

        Ok(page_id)
    }

    pub fn add_image(&mut self, bytes: &[u8]) -> anyhow::Result<ObjectId> {
        match image::guess_format(bytes)? {
            ImageFormat::Jpeg => self.add_jpeg(bytes),
            ImageFormat::Png => self.add_png(bytes),
            _ => anyhow::bail!("unsupported image format"),
        }
    }

    pub fn add_jpeg(&mut self, bytes: &[u8]) -> anyhow::Result<ObjectId> {
        let img = image::load_from_memory(bytes)?;
        let (width, height) = img.dimensions();

        let (cs, bpc) = match img.color() {
            image::ColorType::L8 => ("DeviceGray", 8),
            image::ColorType::L16 => ("DeviceGray", 16),
            image::ColorType::Rgb8 => ("DeviceRGB", 8),
            image::ColorType::Rgb16 => ("DeviceRGB", 16),
            _ => anyhow::bail!("unsupported color type: {:?}", img.color()),
        };

        let page_id = self.add_page(width, height)?;

        let img_stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Filter" => "DCTDecode",
                "BitsPerComponent" => bpc,
                "ColorSpace" => cs,
                "Length" => bytes.len() as u16,
                "Width" => width,
                "Height" =>  height,
            },
            bytes.into(),
        );

        self.doc.insert_image(
            page_id,
            img_stream,
            (0.0, 0.0),
            (width as f32, height as f32),
        )?;

        Ok(page_id)
    }

    pub fn add_png(&mut self, bytes: &[u8]) -> anyhow::Result<ObjectId> {
        let info = crate::png::get_info(bytes)?;

        let bytes = if info.interlace || info.color_type >= 4 {
            let img = image::load_from_memory(bytes)?;
            let mut result = Vec::new();

            let mut writer = Cursor::new(&mut result);

            match info.color_type {
                4 => match info.depth {
                    8 => DynamicImage::ImageLuma8(img.into_luma8()),
                    16 => DynamicImage::ImageLuma16(img.into_luma16()),
                    _ => anyhow::bail!(""),
                },
                6 => match info.depth {
                    8 => DynamicImage::ImageRgb8(img.into_rgb8()),
                    16 => DynamicImage::ImageRgb16(img.into_rgb16()),
                    _ => anyhow::bail!(""),
                },
                _ => img,
            }
            .write_to(&mut writer, ImageFormat::Png)?;
            result
        } else {
            bytes.into()
        };

        let colors = if let 0 | 3 | 4 = info.color_type {
            1
        } else {
            3
        };

        let idat = crate::png::get_idat(&bytes[..])?;

        let cs: Object = match info.color_type {
            0 | 2 | 4 | 6 => {
                if let Some(raw) = info.icc {
                    let icc_id = self.doc.add_object(
                        Stream::new(
                            dictionary!{
                                "N" => colors,
                                "Alternate" => if let 0 | 4 = info.color_type { "DeviceGray" } else { "DeviceRGB" },
                                "Length" => raw.len() as u32,
                                "Filter" => "FlateDecode"
                            },
                            raw
                        )
                    );
                    vec!["ICCBased".into(), icc_id.into()].into()
                } else {
                    if let 0 | 4 = info.color_type {
                        "DeviceGray"
                    } else {
                        "DeviceRGB"
                    }
                    .into()
                }
            }

            3 => {
                let palette = info.palette.unwrap();
                vec![
                    "Indexed".into(),
                    "DeviceRGB".into(),
                    (palette.1 - 1).into(),
                    Object::String(palette.0, StringFormat::Hexadecimal),
                ]
                .into()
            }

            _ => anyhow::bail!("unexpected color type found: {}", info.color_type),
        };

        let page_id = self.add_page(info.width, info.height)?;

        let img_stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Filter" => "FlateDecode",
                "BitsPerComponent" => info.depth,
                "Length" => idat.len() as u32,
                "Width" => info.width,
                "Height" => info.height,
                "DecodeParms" => dictionary!{
                    "BitsPerComponent" => info.depth,
                    "Predictor" => 15,
                    "Columns" => info.width,
                    "Colors" => colors
                },
                "ColorSpace" => cs,
            },
            idat,
        );

        self.doc.insert_image(
            page_id,
            img_stream,
            (0.0, 0.0),
            (info.width as f32, info.height as f32),
        )?;

        Ok(page_id)
    }

    pub fn move_page(&mut self, from: usize, to: usize) -> anyhow::Result<()> {
        let mut pages = self.get_pages();

        let Some(removed) = pages.remove(from) else {
            anyhow::bail!("page {} not found in document", from);
        };
        pages.insert(to, removed);

        Ok(())
    }

    pub fn remove_pages(&mut self, pages: &[u32]) {
        self.doc.delete_pages(pages)
    }

    pub fn prune(&mut self) {
        let _ = self.doc.prune_objects();
        let _ = self.doc.renumber_objects();
    }

    pub fn save<P: AsRef<Path>>(mut self, path: P) -> anyhow::Result<()> {
        self.doc.save(path)?;
        Ok(())
    }

    pub fn to_bytes(mut self) -> anyhow::Result<Vec<u8>> {
        let mut result = Vec::new();
        self.doc.save_to(&mut result)?;

        Ok(result)
    }
}
