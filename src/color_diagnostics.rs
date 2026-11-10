//! Stable diagnostic tags keep the engine's safety decisions independent of UI language.
const PROFILE: &str = "[IMBC_WEBP_PROFILE]";
const JXL: &str = "[IMBC_WEBP_JXL_COLOR]";
const PRECISION: &str = "[IMBC_WEBP_RGB_PRECISION]";

pub fn profile(model: &[u8; 4], jxl_decoder: bool) -> String {
    let tag = if jxl_decoder { JXL } else { PROFILE };
    format!(
        "{tag}{}\n{}",
        String::from_utf8_lossy(model),
        message(0, jxl_decoder, false)
    )
}
pub fn precision(max_error: f64) -> String {
    format!("{PRECISION}max={max_error:.8}\n{}", message(0, false, true))
}
pub fn localize(lang: i32, detail: &str) -> String {
    let (tag, body) = detail.split_once('\n').unwrap_or((detail, ""));
    let (result, original) = if let Some(model) = tag.strip_prefix(JXL) {
        (
            format!("WebP / ICC {model}: {}", message(lang, true, false)),
            message(0, true, false),
        )
    } else if let Some(model) = tag.strip_prefix(PROFILE) {
        (
            format!("WebP / ICC {model}: {}", message(lang, false, false)),
            message(0, false, false),
        )
    } else if let Some(error) = tag.strip_prefix(PRECISION) {
        (
            format!("{error}: {}", message(lang, false, true)),
            message(0, false, true),
        )
    } else {
        // Other engine diagnostics remain verbatim, including cleanup details.
        return detail.into();
    };
    // Job cleanup may append diagnostics after the message; never swallow those.
    result + body.strip_prefix(original).unwrap_or(body)
}
fn message(lang: i32, jxl: bool, precision: bool) -> &'static str {
    let lang = lang.clamp(0, 3) as usize;
    if precision {
        return [
            "無損像素驗證失敗：RGB 色彩轉換後的樣本無法由 8-bit WebP 精確表示，未保存輸出。要保留無損請選 PNG／TIFF／JXL；若接受量化／有損，請自行關閉「WebP 真正無損」再轉換。設定未自動變更。",
            "无损像素验证失败：RGB 色彩转换后的样本无法由 8-bit WebP 精确表示，未保存输出。要保留无损请选择 PNG／TIFF／JXL；若接受量化／有损，请自行关闭「WebP 真正无损」再转换。设置未自动更改。",
            "Lossless pixel verification failed: the RGB-converted samples cannot be represented exactly by 8-bit WebP. No output was saved. Use PNG/TIFF/JXL for lossless output; if quantization/loss is acceptable, manually disable WebP lossless (8-bit only) and retry. Settings were not changed.",
            "可逆画素検証に失敗しました。RGB 変換後の値を 8-bit WebP で正確に表せないため、出力を保存していません。可逆出力には PNG／TIFF／JXL を選択してください。量子化・非可逆変換を許容する場合は「WebP 可逆（8-bitのみ）」を手動でオフにしてください。設定は変更していません。",
        ][lang];
    }
    if jxl {
        [
            "WebP 只能搭配 RGB ICC。此 JXL 的灰階／非 RGB 色彩資訊由解碼器提供，可能是色彩定義合成的 ICC，不能斷定來源真的內嵌 ICC；目前引擎未提供可區分的標記。為避免丟失色彩定義，未保存輸出。要保留無損請選 PNG／TIFF／JXL；若接受色彩轉換及量化／有損，請明確選 RGB 目標 ICC，並自行關閉「WebP 真正無損」。設定未自動變更。",
            "WebP 只能搭配 RGB ICC。此 JXL 的灰阶／非 RGB 色彩信息由解码器提供，可能是色彩定义合成的 ICC，不能断定来源真的内嵌 ICC；当前引擎未提供可区分的标记。为避免丢失色彩定义，未保存输出。要保留无损请选择 PNG／TIFF／JXL；若接受色彩转换及量化／有损，请明确选择 RGB 目标 ICC，并自行关闭「WebP 真正无损」。设置未自动更改。",
            "WebP requires an RGB ICC profile. This JXL's grayscale/non-RGB color information is supplied by the decoder and may be synthesized from its color definition; it does not prove an embedded ICC. The current engine exposes no provenance flag. No output was saved, to avoid losing the color definition. Use PNG/TIFF/JXL for lossless output; if color conversion and quantization/loss are acceptable, explicitly select an RGB target ICC and manually disable WebP lossless (8-bit only). Settings were not changed.",
            "WebP には RGB ICC が必要です。この JXL のグレー／非 RGB 色情報はデコーダーが返したもので、色定義から生成された ICC の可能性があります。埋め込み ICC とは断定できず、現在のエンジンは区別するフラグを公開していません。色定義を失わないため出力を保存していません。可逆出力には PNG／TIFF／JXL を選択してください。色変換・量子化・非可逆変換を許容する場合は RGB ターゲット ICC を明示的に選び、「WebP 可逆（8-bitのみ）」を手動でオフにしてください。設定は変更していません。",
        ][lang]
    } else {
        [
            "WebP 只能搭配 RGB ICC；目前描述檔為灰階／非 RGB 模型，未保存輸出。要保留無損請選 PNG／TIFF／JXL；若接受色彩轉換及量化／有損，請明確選 RGB 目標 ICC，並自行關閉「WebP 真正無損」。只選 RGB 目標不保證能通過真正無損驗證。設定未自動變更。",
            "WebP 只能搭配 RGB ICC；当前描述文件为灰阶／非 RGB 模型，未保存输出。要保留无损请选择 PNG／TIFF／JXL；若接受色彩转换及量化／有损，请明确选择 RGB 目标 ICC，并自行关闭「WebP 真正无损」。只选 RGB 目标不保证通过真正无损验证。设置未自动更改。",
            "WebP requires an RGB ICC profile; the current profile has a grayscale/non-RGB model. No output was saved. Use PNG/TIFF/JXL for lossless output; if color conversion and quantization/loss are acceptable, explicitly select an RGB target ICC and manually disable WebP lossless (8-bit only). Selecting an RGB target alone does not guarantee lossless verification. Settings were not changed.",
            "WebP には RGB ICC が必要です。現在のプロファイルはグレー／非 RGB のため、出力を保存していません。可逆出力には PNG／TIFF／JXL を選択してください。色変換・量子化・非可逆変換を許容する場合は RGB ターゲット ICC を明示的に選び、「WebP 可逆（8-bitのみ）」を手動でオフにしてください。RGB の選択だけでは可逆検証の成功を保証しません。設定は変更していません。",
        ][lang]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_color_guidance_is_localized_without_losing_model_or_error() {
        for language in 0..4 {
            for jxl in [false, true] {
                let text = localize(language, &profile(b"GRAY", jxl));
                assert!(text.contains("GRAY") && text.contains("WebP") && text.contains("RGB"));
                assert!(!text.contains("[IMBC_"));
                assert!(text.contains(["關閉", "关闭", "disable", "オフ"][language as usize]));
                assert!(text.contains("PNG") && text.contains("TIFF") && text.contains("JXL"));
            }
            assert!(localize(language, &precision(0.0018616)).contains("max=0.00186160"));
        }
        assert_eq!(
            localize(2, "unrelated stderr\ncleanup warning"),
            "unrelated stderr\ncleanup warning"
        );
        let with_cleanup = profile(b"GRAY", true) + "；cleanup warning remains visible";
        assert!(localize(2, &with_cleanup).ends_with("；cleanup warning remains visible"));
    }
}
