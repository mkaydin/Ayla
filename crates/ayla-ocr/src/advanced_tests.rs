use crate::knowledge;
use crate::output::PageResult;
use crate::pdf_renderer::{self, OutputFormat, RenderConfig};
use crate::{LayoutSource, OcrPageResult};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[test]
fn golden_pdf_matrix_covers_advanced_document_shapes() {
    let temp = std::env::temp_dir().join(format!("ayla-ocr-golden-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).unwrap();
    let pdf = temp.join("golden-matrix.pdf");

    let mut columns = String::new();
    for row in 0..8 {
        let y = 740 - row * 24;
        columns.push_str(&format!(
            "BT /F1 10 Tf 40 {y} Td (Left column evidence row {row}) Tj ET\n\
             BT /F1 10 Tf 350 {y} Td (Right column evidence row {row}) Tj ET\n"
        ));
    }
    let cases = vec![
        // Raster/scanned pages and blank no-text-layer pages both intentionally
        // lack PDF text objects, but remain distinct golden inputs.
        "0.2 g 40 80 530 640 re f 0.8 g 55 620 480 8 re f 55 590 420 8 re f".into(),
        columns,
        "BT /F1 12 Tf 50 720 Td (Table 1: Accuracy by model) Tj ET\nBT /F1 10 Tf 50 690 Td (Model A 94 percent Model B 91 percent) Tj ET".into(),
        "BT /F1 12 Tf 50 720 Td (Equation: E = mc^2) Tj ET\nBT /F1 10 Tf 50 690 Td (Energy mass relationship) Tj ET".into(),
        "BT /F1 12 Tf 50 720 Td (Figure 1: Experimental setup) Tj ET\n0 0 0 RG 50 300 450 350 re S".into(),
        String::new(),
    ];
    std::fs::write(&pdf, make_pdf(&cases)).unwrap();

    let rendered = pdf_renderer::render_pages(
        &pdf,
        1,
        0,
        &RenderConfig {
            library_path: None,
            dpi: 36.0,
            max_width: 306,
            format: OutputFormat::Png,
            grayscale: true,
            sharpen: false,
            contrast: false,
        },
        &temp.join("pages"),
    )
    .unwrap();

    assert_eq!(rendered.len(), 6);
    assert_eq!(
        rendered[0].layout.source,
        LayoutSource::None,
        "scanned page"
    );
    assert_eq!(rendered[1].layout.columns, 2, "multi-column page");
    assert!(rendered[2]
        .layout
        .figures
        .iter()
        .any(|figure| figure.caption.starts_with("Table 1")));
    assert_eq!(
        rendered[3].layout.source,
        LayoutSource::PdfText,
        "equation page"
    );
    assert!(rendered[4]
        .layout
        .figures
        .iter()
        .any(|figure| figure.caption.starts_with("Figure 1")));
    assert_eq!(
        rendered[5].layout.source,
        LayoutSource::None,
        "no-text-layer page"
    );
    assert!(rendered.iter().all(|page| page.image_path.is_file()));

    std::fs::remove_dir_all(temp).unwrap();
}

#[tokio::test]
async fn completed_page_is_reopened_without_a_second_model_request() {
    let temp = std::env::temp_dir().join(format!("ayla-ocr-resume-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).unwrap();
    let pdf = temp.join("resume.pdf");
    std::fs::write(
        &pdf,
        make_pdf(&["BT /F1 12 Tf 50 720 Td (Resume cache fixture) Tj ET".into()]),
    )
    .unwrap();

    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping local HTTP cache integration in a network-restricted sandbox");
            std::fs::remove_dir_all(temp).unwrap();
            return;
        }
        Err(error) => panic!("could not bind cache integration listener: {error}"),
    };
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let request_counter = requests.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        request_counter.fetch_add(1, Ordering::SeqCst);
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if complete_http_request(&request) {
                break;
            }
        }
        let body = serde_json::json!({
            "response": "# Resume fixture\nThis recognized scientific page contains enough distinct evidence, measurements, symbols, and explanatory language to pass OCR quality checks.",
            "done": true
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let options = crate::OcrOptions {
        model: "fixture-vision".into(),
        ollama_url: url,
        dpi: 36.0,
        max_width: 306,
        sharpen: false,
        retries: 1,
        request_timeout_secs: 5,
        resume_cache: true,
        cache_dir: Some(temp.join("cache")),
        ..Default::default()
    };
    let first = crate::extract_pdf(&pdf, options.clone(), None)
        .await
        .unwrap();
    server.join().unwrap();
    let second = crate::extract_pdf(&pdf, options, None).await.unwrap();

    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(first.performance.pages_recognized, 1);
    assert_eq!(first.performance.pages_from_cache, 0);
    assert_eq!(second.performance.pages_recognized, 0);
    assert_eq!(second.performance.pages_from_cache, 1);
    assert!(second.page_results[0].cached);
    assert_eq!(first.text, second.text);
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn large_document_knowledge_assembly_has_bounded_output_and_runtime() {
    const PAGE_COUNT: usize = 5_000;
    let pages = (1..=PAGE_COUNT)
        .map(|page| PageResult {
            page_num: page as u16,
            text: format!(
                "# Section {page}\nFact {page}: measured value is {}.\n",
                page * 17
            ),
            image_path: String::new(),
        })
        .collect::<Vec<_>>();
    let diagnostics = pages
        .iter()
        .map(|page| OcrPageResult {
            page: page.page_num,
            chars: page.text.chars().count(),
            columns: if page.page_num % 7 == 0 { 2 } else { 1 },
            headings: vec![format!("Section {}", page.page_num)],
            figures: vec![],
            layout_source: LayoutSource::PdfText,
            warnings: vec![],
            cached: page.page_num % 3 == 0,
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    let rss_before = resident_set_kib();
    let package = knowledge::build_knowledge_package(&pages, &diagnostics, "large.pdf");
    let elapsed = started.elapsed();
    let rss_growth_kib = resident_set_kib()
        .zip(rss_before)
        .map(|(after, before)| after.saturating_sub(before));
    let serialized_bytes = serde_json::to_vec(&package).unwrap().len();

    assert_eq!(package.entities.len(), PAGE_COUNT * 2 + 1);
    assert_eq!(package.relations.len(), PAGE_COUNT * 2);
    assert!(package.knowledge_base_markdown.contains("Fact 5000"));
    assert!(serialized_bytes < 32 * 1024 * 1024);
    assert!(
        elapsed.as_secs() < 10,
        "5,000-page deterministic assembly took {elapsed:?}"
    );
    if let Some(growth) = rss_growth_kib {
        assert!(
            growth < 256 * 1024,
            "5,000-page deterministic assembly grew RSS by {growth} KiB"
        );
    }
}

fn complete_http_request(request: &[u8]) -> bool {
    let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    request.len() >= header_end + 4 + content_length
}

fn resident_set_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.trim();
        value.split_whitespace().next()?.parse().ok()
    })
}

fn make_pdf(page_contents: &[String]) -> Vec<u8> {
    let font_id = 3 + page_contents.len() * 2;
    let kids = (0..page_contents.len())
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{kids}] /Count {} >>",
            page_contents.len()
        )
        .into_bytes(),
    ];
    for (index, contents) in page_contents.iter().enumerate() {
        let content_id = 4 + index * 2;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        let mut stream = format!("<< /Length {} >>\nstream\n", contents.len()).into_bytes();
        stream.extend_from_slice(contents.as_bytes());
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
    }
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());

    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}
