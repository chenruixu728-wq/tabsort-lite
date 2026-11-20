use anyhow::{bail, Context, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};
use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;

pub struct Model {
    session: Session,
    tokenizer: Tokenizer,
    cache: HashMap<String, Vec<f32>>,
    language_axis: Option<Vec<f32>>,
    e5: bool,
    uses_type_ids: bool,
}

impl Model {
    pub fn load(directory: &Path) -> Result<Self> {
        let dll = directory.join("onnxruntime.dll");
        let e5_model = directory.join("e5-small-int8.onnx");
        let e5 = e5_model.is_file();
        let model = if e5 {
            e5_model
        } else {
            directory.join("bge-small-zh-int8.onnx")
        };
        let tokenizer = directory.join("tokenizer.json");
        if !dll.is_file() || !model.is_file() || !tokenizer.is_file() {
            bail!("本地分类组件不完整。请使用完整的 TabSort Lite 文件夹。");
        }
        let path: Vec<_> = dll.as_os_str().encode_wide().chain([0]).collect();
        // Check loader errors before entering the inference wrapper's global initialization.
        unsafe {
            let library = LoadLibraryW(path.as_ptr());
            if library.is_null() {
                bail!(
                    "无法加载本地推理组件。请检查 Microsoft Visual C++ x64 运行库。\n{}",
                    std::io::Error::last_os_error()
                );
            }
            FreeLibrary(library);
        }
        let mut tokenizer = Tokenizer::from_file(tokenizer).map_err(anyhow::Error::msg)?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 64,
                ..Default::default()
            }))
            .map_err(anyhow::Error::msg)?;
        let mut padding = tokenizer.get_padding().cloned().unwrap_or_default();
        if e5 {
            padding.pad_token = "<pad>".into();
            padding.pad_id = tokenizer
                .token_to_id("<pad>")
                .context("模型分词器缺少填充标记")?;
        }
        tokenizer.with_padding(Some(PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..padding
        }));
        ort::init_from(dll.to_string_lossy()).commit()?;
        let threads = std::thread::available_parallelism().map_or(2, |count| count.get().min(4));
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(threads)?
            .commit_from_file(model)
            .context("无法加载本地向量模型")?;
        let uses_type_ids = session
            .inputs
            .iter()
            .any(|input| input.name == "token_type_ids");
        Ok(Self {
            session,
            tokenizer,
            cache: HashMap::new(),
            language_axis: None,
            e5,
            uses_type_ids,
        })
    }

    pub fn embed_topics(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = self.embed(texts)?;
        let mixed = texts.iter().any(|text| has_han(text))
            && texts
                .iter()
                .any(|text| !has_han(text) && text.bytes().any(|byte| byte.is_ascii_alphabetic()));
        if !self.e5 || !mixed {
            return Ok(vectors);
        }

        if self.language_axis.is_none() {
            // Each pair expresses the same meaning. Their differences isolate the
            // language signal without using any of the current tabs as examples.
            let anchors = [
                ("Current status and next steps", "当前情况和后续步骤"),
                (
                    "Questions and answers on a related topic",
                    "相关主题的常见问题和解答",
                ),
                ("Examples and practical recommendations", "示例与实践建议"),
                (
                    "Options for planning the next activity",
                    "如何计划下一项活动",
                ),
                ("The main results and conclusions", "主要结果和结论"),
                ("Useful resources for further reading", "可供延伸阅读的资料"),
                ("A comparison of different approaches", "对不同方法的比较"),
                ("An overview and important details", "基本概述和重要细节"),
            ];
            let phrases = anchors
                .into_iter()
                .flat_map(|(en, zh)| [en.to_owned(), zh.to_owned()])
                .collect::<Vec<_>>();
            let samples = self.embed(&phrases)?;
            let mut axis = vec![0.0; samples[0].len()];
            for pair in samples.chunks_exact(2) {
                for ((total, en), zh) in axis.iter_mut().zip(&pair[0]).zip(&pair[1]) {
                    *total += en - zh;
                }
            }
            normalize(&mut axis);
            self.language_axis = Some(axis);
        }
        if let Some(axis) = &self.language_axis {
            for vector in &mut vectors {
                let projection = dot(vector, axis);
                for (value, axis) in vector.iter_mut().zip(axis) {
                    *value -= projection * axis;
                }
                normalize(vector);
            }
        }
        Ok(vectors)
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if self.cache.len() + texts.len() > 5000 {
            self.cache.clear();
        }
        let mut missing = texts
            .iter()
            .filter(|text| !self.cache.contains_key(*text))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        missing.dedup();
        for batch in missing.chunks(16) {
            let tokens = self
                .tokenizer
                .encode_batch(
                    batch
                        .iter()
                        .map(|text| {
                            if self.e5 {
                                format!("query: {text}").into()
                            } else {
                                std::borrow::Cow::Borrowed(text.as_str())
                            }
                        })
                        .collect::<Vec<_>>(),
                    true,
                )
                .map_err(anyhow::Error::msg)?;
            let width = tokens[0].len();
            let shape = [tokens.len(), width];
            let ids: Vec<_> = tokens
                .iter()
                .flat_map(|encoding| encoding.get_ids())
                .map(|&id| i64::from(id))
                .collect();
            let masks: Vec<_> = tokens
                .iter()
                .flat_map(|encoding| encoding.get_attention_mask())
                .map(|&mask| i64::from(mask))
                .collect();
            let ids = Tensor::from_array((shape, ids))?;
            let masks = Tensor::from_array((shape, masks))?;
            let outputs = if self.uses_type_ids {
                let type_ids: Vec<_> = tokens
                    .iter()
                    .flat_map(|encoding| encoding.get_type_ids())
                    .map(|&id| i64::from(id))
                    .collect();
                self.session.run(ort::inputs! {
                    "input_ids" => ids,
                    "attention_mask" => masks,
                    "token_type_ids" => Tensor::from_array((shape, type_ids))?,
                })?
            } else {
                self.session
                    .run(ort::inputs! { "input_ids" => ids, "attention_mask" => masks })?
            };
            let (dimensions, values) = outputs[0].try_extract_tensor::<f32>()?;
            if dimensions.len() != 3
                || dimensions[0] != batch.len() as i64
                || dimensions[1] != width as i64
            {
                bail!("本地模型输出形状无效");
            }
            let features = dimensions[2] as usize;
            for (index, &text) in batch.iter().enumerate() {
                let start = index * width * features;
                let mut vector = if self.e5 {
                    // With L2 normalization, summing the unmasked tokens equals mean pooling.
                    let mut sum = vec![0.0; features];
                    for (row, &mask) in values[start..start + width * features]
                        .chunks_exact(features)
                        .zip(tokens[index].get_attention_mask())
                    {
                        if mask != 0 {
                            for (total, value) in sum.iter_mut().zip(row) {
                                *total += value;
                            }
                        }
                    }
                    sum
                } else {
                    values[start..start + features].to_vec()
                };
                normalize(&mut vector);
                self.cache.insert(text.clone(), vector);
            }
        }
        texts
            .iter()
            .map(|text| self.cache.get(text).cloned().context("缺少标签页向量"))
            .collect()
    }
}

fn has_han(text: &str) -> bool {
    text.chars()
        .any(|ch| matches!(ch, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'))
}

pub fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}
