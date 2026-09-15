//! Rime `.dict.yaml` → 青简 TSV，**开发测试词库专用**。
//!
//! 典型输入是雾凇拼音（[iDvel/rime-ice](https://github.com/iDvel/rime-ice)）的 `cn_dicts/base.dict.yaml`
//! （`报错 bao cuo 74910` 这类词自建词库没收），可再叠 `ext.dict.yaml`。只做格式转换，同词同音取最大权重。
//!
//! 雾凇是 GPL-3.0-only，与自建词库的数据许可策略不同，因此只当开发测试词库、不随产品发布；输出默认写
//! `data/generated/`（gitignore），CLI 用 `--dict`、Fcitx5 用 `QINGJIAN_SHARE_DIR` 指过去。见 `docs/design/landscape.md`。

use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use qingjian_dictionary::import::rime;

use crate::error::ConvertError;

/// 把 `inputs` 里的 Rime 词库合并成一份青简 TSV 写到 `output`。同词同音取最大权重，按（词，音节）排序。
pub fn convert(inputs: &[PathBuf], output: &Path) -> Result<(), ConvertError> {
    let mut merged: BTreeMap<(String, String), u32> = BTreeMap::new();
    for input in inputs {
        let source = std::fs::read_to_string(input)?;
        if !rime::looks_like_rime(&source) {
            return Err(ConvertError::Format {
                path: input.clone(),
                line: 1,
                reason: "缺 YAML 头，不像 Rime .dict.yaml".into(),
            });
        }
        let parsed = rime::to_tsv(&source);
        for line in parsed.tsv.lines() {
            let mut fields = line.split('\t');
            let (Some(word), Some(pinyin)) = (fields.next(), fields.next()) else {
                continue;
            };
            let weight = fields
                .next()
                .and_then(|w| w.parse::<u32>().ok())
                .unwrap_or(1);
            let entry = merged
                .entry((word.to_owned(), pinyin.to_owned()))
                .or_insert(0);
            *entry = (*entry).max(weight);
        }
    }

    let mut file = BufWriter::new(std::fs::File::create(output)?);
    writeln!(
        file,
        "# 由 qingjian-dict-convert rime 从 Rime .dict.yaml 生成（雾凇拼音 iDvel/rime-ice，GPL-3.0-only）。\
         开发测试词库，不随产品发布。词\\t音节\\t权重"
    )?;
    let entries = merged.len();
    for ((word, pinyin), weight) in &merged {
        writeln!(file, "{word}\t{pinyin}\t{weight}")?;
    }
    file.flush()?;
    tracing::info!(path = %output.display(), entries, "写出完成");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_and_keeps_max_weight() {
        let dir = std::env::temp_dir().join(format!("qingjian-rime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.dict.yaml");
        let ext = dir.join("ext.dict.yaml");
        std::fs::write(
            &base,
            "---\nname: base\n...\n报错\tbao cuo\t74910\n搞错\tgao cuo\t12935\n",
        )
        .unwrap();
        std::fs::write(
            &ext,
            "---\nname: ext\n...\n报错\tbao cuo\t100\n爆粗\tbao cu\t1170\n",
        )
        .unwrap();
        let output = dir.join("rime-ice.tsv");
        convert(&[base, ext], &output).unwrap();
        let text = std::fs::read_to_string(&output).unwrap();
        assert!(text.contains("报错\tbao cuo\t74910\n"));
        assert!(text.contains("爆粗\tbao cu\t1170\n"));
        assert!(!text.contains("报错\tbao cuo\t100\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_non_rime_input() {
        let dir = std::env::temp_dir().join(format!("qingjian-rime-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("dict.tsv");
        std::fs::write(&input, "报错\tbao cuo\t74910\n").unwrap();
        let error = convert(&[input], &dir.join("out.tsv")).unwrap_err();
        assert!(matches!(error, ConvertError::Format { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
