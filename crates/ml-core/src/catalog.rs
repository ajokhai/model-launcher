//! Open models Model Launcher knows how to download and run. Every file is a direct Hugging Face
//! download; sizes are for planning and are replaced by the real size once a download starts.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Text (and, with a projector, image) chat through llama.cpp.
    Chat,
    /// Speech to text through whisper.cpp.
    Transcribe,
    /// Text to speech through llama.cpp's TTS.
    Speak,
    /// Text to image through stable-diffusion.cpp.
    Image,
    /// Text to video through stable-diffusion.cpp (Wan).
    Video,
}

impl Kind {
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Chat => "Chat",
            Kind::Transcribe | Kind::Speak => "Voice",
            Kind::Image => "Image",
            Kind::Video => "Video",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Weights,
    /// Lets a chat model read images (llama.cpp's mmproj).
    Projector,
    /// Turns a TTS model's tokens into audio.
    Vocoder,
    /// Reads the prompt for a video model (Wan's umt5).
    TextEncoder,
    /// Turns a video model's latents into frames.
    Vae,
}

#[derive(Clone, Debug, Serialize)]
pub struct File {
    pub repo: &'static str,
    pub name: &'static str,
    pub size_gb: f64,
    pub role: Role,
}

impl File {
    pub fn url(&self) -> String {
        format!("https://huggingface.co/{}/resolve/main/{}", self.repo, self.name)
    }

    /// The name it's saved under (a file can sit in a subfolder of its repo).
    pub fn local_name(&self) -> &'static str {
        self.name.rsplit('/').next().unwrap_or(self.name)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub maker: &'static str,
    pub blurb: &'static str,
    pub vision: bool,
    pub files: Vec<File>,
}

impl Model {
    pub fn size_gb(&self) -> f64 {
        self.files.iter().map(|f| f.size_gb).sum()
    }
}

fn f(repo: &'static str, name: &'static str, size_gb: f64, role: Role) -> File {
    File { repo, name, size_gb, role }
}

pub fn all() -> Vec<Model> {
    use Kind::*;
    use Role::*;
    vec![
        Model {
            id: "qwen3-vl-8b", name: "Qwen3-VL 8B", kind: Chat, maker: "Qwen", vision: true,
            blurb: "Strong all-rounder that can also read images and screenshots. Good tool use.",
            files: vec![
                f("Qwen/Qwen3-VL-8B-Instruct-GGUF", "Qwen3VL-8B-Instruct-Q4_K_M.gguf", 5.03, Weights),
                f("Qwen/Qwen3-VL-8B-Instruct-GGUF", "mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf", 0.75, Projector),
            ],
        },
        Model {
            id: "qwen3-vl-4b", name: "Qwen3-VL 4B", kind: Chat, maker: "Qwen", vision: true,
            blurb: "Half the size of the 8B with vision. Quick on any Apple Silicon Mac.",
            files: vec![
                f("Qwen/Qwen3-VL-4B-Instruct-GGUF", "Qwen3VL-4B-Instruct-Q4_K_M.gguf", 2.50, Weights),
                f("Qwen/Qwen3-VL-4B-Instruct-GGUF", "mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf", 0.45, Projector),
            ],
        },
        Model {
            id: "internvl3-14b", name: "InternVL3 14B", kind: Chat, maker: "OpenGVLab", vision: true,
            blurb: "Bigger vision model trained on screenshots and GUIs. Wants a 12 GB budget.",
            files: vec![
                f("ggml-org/InternVL3-14B-Instruct-GGUF", "InternVL3-14B-Instruct-Q4_K_M.gguf", 8.99, Weights),
                f("ggml-org/InternVL3-14B-Instruct-GGUF", "mmproj-InternVL3-14B-Instruct-Q8_0.gguf", 0.38, Projector),
            ],
        },
        Model {
            id: "qwen3-8b", name: "Qwen3 8B", kind: Chat, maker: "Qwen", vision: false,
            blurb: "Text only, with a thinking mode for harder questions.",
            files: vec![f("Qwen/Qwen3-8B-GGUF", "Qwen3-8B-Q4_K_M.gguf", 5.03, Weights)],
        },
        Model {
            id: "qwen3-4b", name: "Qwen3 4B", kind: Chat, maker: "Qwen", vision: false,
            blurb: "Small, fast text model for everyday questions.",
            files: vec![f("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf", 2.50, Weights)],
        },
        Model {
            id: "qwen3-14b", name: "Qwen3 14B", kind: Chat, maker: "Qwen", vision: false,
            blurb: "Noticeably smarter, and needs a Mac with 24 GB or more.",
            files: vec![f("Qwen/Qwen3-14B-GGUF", "Qwen3-14B-Q4_K_M.gguf", 9.00, Weights)],
        },
        Model {
            id: "gemma-3-4b", name: "Gemma 3 4B", kind: Chat, maker: "Google", vision: true,
            blurb: "Google's small open model, with vision.",
            files: vec![
                f("ggml-org/gemma-3-4b-it-GGUF", "gemma-3-4b-it-Q4_K_M.gguf", 2.49, Weights),
                f("ggml-org/gemma-3-4b-it-GGUF", "mmproj-model-f16.gguf", 0.85, Projector),
            ],
        },
        Model {
            id: "gemma-3-12b", name: "Gemma 3 12B", kind: Chat, maker: "Google", vision: true,
            blurb: "Bigger Gemma with vision. Wants 24 GB or more.",
            files: vec![
                f("ggml-org/gemma-3-12b-it-GGUF", "gemma-3-12b-it-Q4_K_M.gguf", 7.30, Weights),
                f("ggml-org/gemma-3-12b-it-GGUF", "mmproj-model-f16.gguf", 0.85, Projector),
            ],
        },
        Model {
            id: "qwen2.5-coder-7b", name: "Qwen2.5 Coder 7B", kind: Chat, maker: "Qwen", vision: false,
            blurb: "Tuned for writing and explaining code.",
            files: vec![f("Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", "qwen2.5-coder-7b-instruct-q4_k_m.gguf", 4.68, Weights)],
        },
        Model {
            id: "llama-3.2-3b", name: "Llama 3.2 3B", kind: Chat, maker: "Meta", vision: false,
            blurb: "Meta's compact model. Light and fast.",
            files: vec![f("bartowski/Llama-3.2-3B-Instruct-GGUF", "Llama-3.2-3B-Instruct-Q4_K_M.gguf", 2.02, Weights)],
        },
        Model {
            id: "qwen2.5-1.5b", name: "Qwen2.5 1.5B", kind: Chat, maker: "Qwen", vision: false,
            blurb: "Tiny and instant. Fine for simple text, not for agents.",
            files: vec![f("Qwen/Qwen2.5-1.5B-Instruct-GGUF", "qwen2.5-1.5b-instruct-q4_k_m.gguf", 1.12, Weights)],
        },
        Model {
            id: "gpt-oss-20b", name: "gpt-oss 20B", kind: Chat, maker: "OpenAI", vision: false,
            blurb: "OpenAI's open-weight reasoning model. Needs 32 GB or more.",
            files: vec![f("ggml-org/gpt-oss-20b-GGUF", "gpt-oss-20b-MXFP4.gguf", 12.11, Weights)],
        },
        Model {
            id: "whisper-base-en", name: "Whisper Base (English)", kind: Transcribe, maker: "OpenAI", vision: false,
            blurb: "Fast English speech to text.",
            files: vec![f("ggerganov/whisper.cpp", "ggml-base.en.bin", 0.15, Weights)],
        },
        Model {
            id: "whisper-small-en", name: "Whisper Small (English)", kind: Transcribe, maker: "OpenAI", vision: false,
            blurb: "More accurate English speech to text.",
            files: vec![f("ggerganov/whisper.cpp", "ggml-small.en.bin", 0.49, Weights)],
        },
        Model {
            id: "whisper-large-v3-turbo", name: "Whisper Large v3 Turbo", kind: Transcribe, maker: "OpenAI", vision: false,
            blurb: "Best accuracy, 99 languages.",
            files: vec![f("ggerganov/whisper.cpp", "ggml-large-v3-turbo-q5_0.bin", 0.57, Weights)],
        },
        Model {
            id: "outetts-0.2", name: "OuteTTS 0.2", kind: Speak, maker: "OuteAI", vision: false,
            blurb: "Text to speech that runs fully on your Mac.",
            files: vec![
                f("OuteAI/OuteTTS-0.2-500M-GGUF", "OuteTTS-0.2-500M-Q8_0.gguf", 0.54, Weights),
                f("ggml-org/WavTokenizer", "WavTokenizer-Large-75-F16.gguf", 0.13, Vocoder),
            ],
        },
        Model {
            id: "wan2.1-t2v-1.3b", name: "Wan 2.1 1.3B", kind: Video, maker: "Wan (Alibaba)", vision: false,
            blurb: "Short video clips from a prompt. Slow on a laptop: expect minutes per clip.",
            files: vec![
                f("samuelchristlie/Wan2.1-T2V-1.3B-GGUF", "Wan2.1-T2V-1.3B-Q4_K_M.gguf", 0.98, Weights),
                f("city96/umt5-xxl-encoder-gguf", "umt5-xxl-encoder-Q4_K_M.gguf", 3.66, TextEncoder),
                f("Comfy-Org/Wan_2.1_ComfyUI_repackaged", "split_files/vae/wan_2.1_vae.safetensors", 0.25, Vae),
            ],
        },
        Model {
            id: "sd-turbo", name: "SD Turbo", kind: Image, maker: "Stability AI", vision: false,
            blurb: "Makes a 512 px image in a few steps. The quickest image model.",
            files: vec![f("Green-Sky/SD-Turbo-GGUF", "sd_turbo-f16-q8_0.gguf", 2.02, Weights)],
        },
        Model {
            id: "sd-1.5", name: "Stable Diffusion 1.5", kind: Image, maker: "Stability AI", vision: false,
            blurb: "The classic. Slower than Turbo, more detail with 20+ steps.",
            files: vec![f("second-state/stable-diffusion-v1-5-GGUF", "stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf", 1.76, Weights)],
        },
    ]
}

pub fn get(id: &str) -> Option<Model> {
    all().into_iter().find(|m| m.id == id)
}

/// Where a model comes from, for attribution: its license (as its Hugging Face card states it), the
/// original model it was converted from, and the repo the files are downloaded from.
#[derive(Clone, Debug, Serialize)]
pub struct Attribution {
    pub license: &'static str,
    pub license_url: &'static str,
    pub origin: &'static str,
    pub download_repo: &'static str,
}

pub fn attribution(m: &Model) -> Attribution {
    let (license, license_url, origin) = match m.id {
        "qwen3-vl-8b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen3-VL-8B-Instruct"),
        "qwen3-vl-4b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen3-VL-4B-Instruct"),
        "internvl3-14b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "OpenGVLab/InternVL3-14B-Instruct"),
        "qwen3-8b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen3-8B"),
        "qwen3-4b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen3-4B"),
        "qwen3-14b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen3-14B"),
        "gemma-3-4b" => ("Gemma Terms of Use", "https://ai.google.dev/gemma/terms", "google/gemma-3-4b-it"),
        "gemma-3-12b" => ("Gemma Terms of Use", "https://ai.google.dev/gemma/terms", "google/gemma-3-12b-it"),
        "qwen2.5-coder-7b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen2.5-Coder-7B-Instruct"),
        "llama-3.2-3b" => ("Llama 3.2 Community License", "https://www.llama.com/llama3_2/license/", "meta-llama/Llama-3.2-3B-Instruct"),
        "qwen2.5-1.5b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Qwen/Qwen2.5-1.5B-Instruct"),
        "gpt-oss-20b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "openai/gpt-oss-20b"),
        "whisper-base-en" | "whisper-small-en" | "whisper-large-v3-turbo" => ("MIT", "https://opensource.org/license/mit", "openai/whisper"),
        "outetts-0.2" => ("CC BY-NC 4.0 (non-commercial)", "https://creativecommons.org/licenses/by-nc/4.0/", "OuteAI/OuteTTS-0.2-500M"),
        "wan2.1-t2v-1.3b" => ("Apache 2.0", "https://www.apache.org/licenses/LICENSE-2.0", "Wan-AI/Wan2.1-T2V-1.3B"),
        "sd-turbo" => ("Stability AI Community License", "https://stability.ai/community-license-agreement", "stabilityai/sd-turbo"),
        "sd-1.5" => ("CreativeML OpenRAIL-M", "https://huggingface.co/spaces/CompVis/stable-diffusion-license", "runwayml/stable-diffusion-v1-5"),
        _ => ("See the model card", "", ""),
    };
    Attribution { license, license_url, origin, download_repo: m.files[0].repo }
}
