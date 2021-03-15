use chrono::offset::Utc;
use image::{GenericImageView, ImageFormat, DynamicImage};
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};

pub struct Pdf {
    pub pdf: Document,
    pub pages_id: ObjectId,
}

impl From<Document> for Pdf {
    fn from(from: Document) -> Self {
        Self { pages_id: from.catalog().expect("unable to read catalog").get(b"Pages").and_then(Object::as_reference).expect("unable to read pages").to_owned(), pdf: from }
    }
}

impl Pdf {
    pub fn new() -> Self {
        let mut pdf = Document::with_version("1.7");

        let info_id = pdf.add_object(
            dictionary!{
                "CreationDate" => Utc::now(),
                "ModDate" => Utc::now(),
            }
        );

        pdf.trailer.set("Info", info_id);

        let catalog_id = pdf.new_object_id();
        pdf.trailer.set("Root", catalog_id);

        let pages_id = pdf.add_object(
            dictionary!{
                "Type" => "Pages",
                "Count" => 0,
                "Kids" => vec![],
            }
        );

        pdf.objects.insert(
            catalog_id,
            dictionary! {
                "Type" => "Catalog",
                "Pages" => pages_id,
            }.into()
        );

        Self {
            pdf: pdf,
            pages_id: pages_id,
        }
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

        let info = self.pdf.trailer.get(b"Info").and_then(Object::as_reference)?;

        self.pdf
            .get_object_mut(info)
            .and_then(Object::as_dict_mut)?
            .set("Author", Object::String(utfbe_str, StringFormat::Hexadecimal));

        Ok(())
    }

    pub fn add_page(&mut self, width: u32, height: u32) -> anyhow::Result<ObjectId> {
        let page_id = self.pdf.new_object_id();
        let contents_id = self.pdf.add_object(Stream::new(dictionary!{}, vec![]));

        self.pdf.objects.insert(
            page_id,
            dictionary! {
                "Type" => "Page",
                "Parent" => self.pages_id,
                "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
                "Contents" => contents_id,
            }.into()
        );

        let pages = self.pdf.get_object_mut(self.pages_id).and_then(Object::as_dict_mut).unwrap();
        pages.set("Count", pages.get(b"Count").and_then(Object::as_i64).unwrap() + 1);
        pages.get_mut(b"Kids").and_then(Object::as_array_mut).unwrap().push(page_id.into());
        Ok(page_id)
    }

    pub fn add_link(&mut self, link: &str, page_id: ObjectId) -> anyhow::Result<()> {
        let rect = self.pdf.get_object_mut(page_id).and_then(Object::as_dict_mut)?.get(b"MediaBox")?.to_owned();

        let url_id = self.pdf.add_object(
            dictionary!{
                "S" => "URI",
                "URI" => Object::string_literal(link),
            }
        );

        let annot_id = self.pdf.add_object(
            dictionary!{
                "Type" => "Annot",
                "Subtype" => "Link",
                "A" => url_id,
                "Rect" => rect,
                "Border" => vec![0.into(), 0.into(), 0.into()],
                "F" => 4,
            }
        );

        let page = self.pdf.get_object_mut(page_id).and_then(Object::as_dict_mut)?;
        page.set("Annots", vec![annot_id.into()]);
        Ok(())
    }

    pub fn remove_link(&mut self, page_id: ObjectId) -> anyhow::Result<()> {
        self.pdf.get_object_mut(page_id).and_then(Object::as_dict_mut)?.remove(b"Annots");
        Ok(())
    }

    pub fn move_link(&mut self, src: ObjectId, dst: ObjectId) -> anyhow::Result<()> {
        let annots_opt = self.pdf.get_object_mut(src).and_then(Object::as_dict_mut)?.remove(b"Annots");
        if let Some(annots) = annots_opt {
            self.pdf.get_object_mut(dst).and_then(Object::as_dict_mut)?.set("Annots", annots);
        }

        Ok(())
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

        self.pdf.insert_image(page_id, img_stream, (0.0, 0.0), (width.into(), height.into()))?;
        Ok(page_id)
    }

    pub fn add_png(&mut self, bytes: &[u8]) -> anyhow::Result<ObjectId> {
        let info = crate::png::get_info(bytes)?;

        let bytes = if info.interlace || info.color_type >= 4 {
            let img = image::load_from_memory(bytes)?;
            let mut result = Vec::new();

            match info.color_type {
                4 =>  match info.depth {
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
            .write_to(&mut result, ImageFormat::Png)?;
            result
        } else {
            bytes.into()
        };

        let colors = if let 0 | 3 | 4 = info.color_type { 1 } else { 3 };

        let idat = crate::png::get_idat(&bytes[..])?;

        let cs: Object = match info.color_type {
            0 | 2 | 4 | 6 => {
                if let Some(raw) = info.icc {
                    let icc_id = self.pdf.add_object(
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
                    if let 0 | 4 = info.color_type { "DeviceGray" } else { "DeviceRGB" }.into()
                }
            },

            3 => {
                let palette = info.palette.unwrap();
                vec!["Indexed".into(), "DeviceRGB".into(), (palette.1 - 1).into(), Object::String(palette.0, StringFormat::Hexadecimal)].into()
            },

            _ => anyhow::bail!("unexpected color type found: {}", info.color_type),
        };

        let page_id = self.add_page(info.width, info.height)?;
        
        let img_stream = Stream::new(
            dictionary!{
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
            idat
        );

        self.pdf.insert_image(page_id, img_stream, (0.0, 0.0), (info.width.into(), info.height.into()))?;
        Ok(page_id)
    }

    pub fn save<W: std::io::Write>(mut self, target: &mut W) -> anyhow::Result<()> {
        self.pdf.save_to(target)?;
        Ok(())
    }
}
