//! Engine 装配：与 `apps/cli` 同一套加载顺序，路径换成 Linux 约定。
//!
//! 这是 Core 之外第二个知道具体 Translator / Learner 类型的地方（第一个是 CLI、然后是三个平台壳），
//! 装配代码与各壳一样，不往 Core 里塞。

use std::path::Path;
use std::time::Instant;

use qingjian_core::{EmojiTable, Engine, Language};
use qingjian_dictionary::{Dictionary, WordList};
use qingjian_learning::FrequencyLearner;
use qingjian_lm::BigramModel;
use qingjian_platform::{Config, extra_dictionaries};
use qingjian_predict::CloudPredictor;
use qingjian_translate::Glossary;

use crate::paths;

/// 装配结果：Engine 与它用到的配置（每页候选数、云端槽位、翻页键等壳侧参数也从这里取）。
pub struct Assembled {
    pub engine: Engine,
    pub config: Config,
}

/// 组装 Engine。数据文件缺失时尽可能退化（没有释义表就只出候选、没有 LM 就一元整句），
/// 只有主词库找不到才算失败。
pub fn build(
    config_path: Option<&Path>,
    user_dir: Option<&Path>,
    share_dir: &Path,
) -> Result<Assembled, String> {
    let started = Instant::now();
    let config = match config_path {
        Some(path) => Config::load(path).map_err(|error| format!("读取配置失败：{error}"))?,
        None => Config::default(),
    };

    let language: Language = config
        .general
        .learning_language
        .parse()
        .map_err(|_| format!("不认识的学习语言：{}", config.general.learning_language))?;
    if language == Language::Chinese {
        return Err("学习语言不能是中文".to_owned());
    }

    let search = paths::data_search_dirs(share_dir, user_dir);

    let dict_path = paths::find_data_file(&search, "dict.tsv").ok_or_else(|| {
        format!(
            "找不到主词库（{} 下查 dict.tsv / dict.qj）",
            share_dir.display()
        )
    })?;
    let dictionary =
        Dictionary::from_path(&dict_path).map_err(|error| format!("主词库加载失败：{error}"))?;

    let glossary = paths::find_data_file(&search, &format!("glossary-{}.tsv", language.code()))
        .map(|path| Glossary::from_path(language, &path))
        .transpose()
        .map_err(|error| format!("释义表加载失败：{error}"))?;

    let english = paths::find_data_file(&search, "english.tsv")
        .map(|path| WordList::from_path(&path))
        .transpose()
        .map_err(|error| format!("英文词表加载失败：{error}"))?;

    let learner = match user_dir.map(|dir| dir.join("user.tsv")) {
        Some(path) => FrequencyLearner::from_path(path).unwrap_or_else(|error| {
            tracing::warn!(%error, "用户词频读取失败，退回只在内存里学习");
            FrequencyLearner::default()
        }),
        None => FrequencyLearner::default(),
    };

    let mut engine = Engine::new(dictionary);
    if let Some(glossary) = glossary {
        engine = engine.with_translator(Box::new(glossary));
    }
    engine = engine.with_learner(Box::new(learner));

    // 附加词库：随包的领域词库（`<share>/dicts`）与用户导入的（`<data>/dicts`），开关看 [dictionaries]。
    let bundled_dicts = shared_dicts_dir(share_dir);
    let user_dicts = user_dir
        .map(|dir| dir.join("dicts"))
        .filter(|dir| dir.is_dir());
    let extras = extra_dictionaries::load(
        bundled_dicts.as_deref(),
        user_dicts.as_deref(),
        &config.dictionaries,
    );
    if !extras.is_empty() {
        tracing::info!(count = extras.len(), "附加词库已加载");
        engine.set_extra_dictionaries(extras);
    }

    if let Some(words) = english {
        engine = engine.with_english(words);
    }

    // 英→中释义（英文候选的中文 annotation），可选。
    if let Some(path) = paths::find_data_file(&search, "glossary-zh.tsv")
        && let Ok(glossary) = Glossary::from_path(Language::Chinese, &path)
    {
        engine = engine.with_english_translator(Box::new(glossary));
    }

    // emoji 表（Unicode License），可选；中英两张合成一张。
    let mut emoji: Option<EmojiTable> = None;
    for name in ["emoji-zh.tsv", "emoji-en.tsv"] {
        let Some(path) = paths::find_data_file(&search, name) else {
            continue;
        };
        match EmojiTable::from_path(&path) {
            Ok(table) => match &mut emoji {
                Some(all) => all.merge(table),
                None => emoji = Some(table),
            },
            Err(error) => tracing::warn!(file = %path.display(), %error, "emoji 表加载失败，跳过"),
        }
    }
    if let Some(table) = emoji {
        engine = engine.with_emoji(table);
    }

    if let Some(model) = load_language_model(&search)? {
        engine = engine.with_language_model(Box::new(model));
    }

    engine.set_fuzzy(config.fuzzy);
    engine.set_mode_keys(config.shortcut.mode);
    engine.set_shuangpin(config.general.shuangpin());
    engine.set_zhuyin_mode(config.general.zhuyin);
    engine.set_full_width_punctuation(config.general.full_width_punctuation);

    if config.predict.enabled {
        match CloudPredictor::new(&config.predict) {
            Ok(predictor) => engine = engine.with_predictor(Box::new(predictor)),
            Err(error) => tracing::warn!(%error, "云联想初始化失败，本次不启用"),
        }
    }

    tracing::info!(
        dict = %dict_path.display(),
        entries = engine.dictionary().len(),
        total_ms = started.elapsed().as_millis(),
        "Engine 就绪"
    );
    Ok(Assembled { engine, config })
}

fn shared_dicts_dir(share_dir: &Path) -> Option<std::path::PathBuf> {
    let dir = share_dir.join("dicts");
    dir.is_dir().then_some(dir)
}

/// 语言模型可选：打包的 `lm.qj` 优先，否则一元 + 二元两份 TSV；都没有就返回 `None`（Core 退化为一元整句）。
fn load_language_model(search: &[std::path::PathBuf]) -> Result<Option<BigramModel>, String> {
    if let Some(path) = paths::find_data_file(search, "lm.qj") {
        let started = Instant::now();
        let model =
            BigramModel::from_path(&path).map_err(|error| format!("语言模型加载失败：{error}"))?;
        tracing::info!(
            words = model.word_count(),
            load_ms = started.elapsed().as_millis(),
            "语言模型已加载"
        );
        return Ok(Some(model));
    }
    if let (Some(unigram), Some(bigram)) = (
        paths::find_data_file(search, "lm-unigram.tsv"),
        paths::find_data_file(search, "lm-bigram.tsv"),
    ) {
        let started = Instant::now();
        let model = BigramModel::from_paths(&unigram, &bigram)
            .map_err(|error| format!("语言模型加载失败：{error}"))?;
        tracing::info!(
            words = model.word_count(),
            load_ms = started.elapsed().as_millis(),
            "语言模型已加载"
        );
        return Ok(Some(model));
    }
    Ok(None)
}
