//! 語言標籤配對：依 CLDR 語言距離挑最接近的語言碼，規則照 Python 版用的 langcodes 3.5.1 `closest_match`。
//!
//! 可能字體系與地區（likely subtags）、別名交給 ICU4X；距離規則與資料表在本檔。

use icu_locale::{Locale, LocaleCanonicalizer, LocaleExpander, ParseError};

const MAX_DISTANCE: u32 = 25;
const DEFAULT_LANGUAGE_DISTANCE: u32 = 80;
const DEFAULT_SCRIPT_DISTANCE: u32 = 50;
const DEFAULT_TERRITORY_DISTANCE: u32 = 4;

// langcodes LANGUAGE_DISTANCES 裡不超過 MAX_DISTANCE 的項目（期望 支援 距離）；更遠的與預設值一樣過不了門檻。
const CLOSE_PAIRS: &str = "
    aa ssy 4, aae sq 10, aao ar 10, aat sq 10, abh ar 10, abv ar 10, acm ar 10, acq ar 10,
    acw ar 10, acx ar 10, acy ar 10, adf ar 10, aeb ar 10, aec ar 10, af nl 20, afb ar 10,
    aii syr 10, ajp ar 10, aln sq 10, am_Ethi en_Latn 10, apc ar 10, apd ar 10,
    ar_Latn ar_Arab 20, arq ar 10, ars ar 10, ary ar 10, arz ar 10, auz ar 10, avl ar 10,
    ay es 20, ayh ar 10, ayl ar 10, ayn ar 10, ayp ar 10, az_Latn ru_Cyrl 10, azb az 10,
    bal ur 20, bbz ar 10, be ru 20, bhr mg 10, bjn ms 10, bjq mg 10, bmm mg 10,
    bn_Beng en_Latn 10, bn_Latn bn_Beng 20, bo zh 20, bo_Tibt zh_Hans 10, br fr 20, bs hr 4,
    bs sh 4, bs sr 5, btj ms 10, bve ms 10, bvu ms 10, bzc mg 10, ca es 20, cdo zh 10,
    chr en 20, cjy zh 10, co fr 20, coa ms 10, cpx zh 10, crs fr 20, cs sk 20, cy en 20,
    czh zh 10, czo zh 10, da no 8, da nb 8, dty ne 10, dup ms 10, enb kln 10, eu es 20,
    eyo kln 10, ffm ff 10, fo da 20, fub ff 10, fue ff 10, fuf ff 10, fuh ff 10, fui ff 10,
    fuq ff 10, fuv ff 10, fy nl 20, ga en 20, gan zh 10, gax om 10, gd en 20, gkp kpe 10,
    gl es 20, gn es 20, gnw gn 10, gom kok 10, gsw de 4, gu_Latn gu_Gujr 20, gui gn 10,
    gun gn 10, hae om 10, hak zh 10, haw en 20, hi_Latn hi_Deva 20, hji ms 10, hr bs 4,
    hr sh 4, hr sr 5, hsn zh 10, ht fr 20, hy_Armn ru_Cyrl 10, id ms 10, ida luy 10, ikt iu 10,
    is en 20, ja_Hani ja_Jpan 5, ja_Hira ja_Jpan 5, ja_Hira ja_Hrkt 5, ja_Hrkt ja_Jpan 5,
    ja_Kana ja_Jpan 5, ja_Kana ja_Hrkt 5, ja_Latn ja_Jpan 5, jak ms 10, jax ms 10, jv id 20,
    ka_Geor en_Latn 10, km_Khmr en_Latn 10, kn_Knda en_Latn 10, kn_Latn kn_Knda 20,
    ko_Hang ko_Kore 5, ko_Hani ko_Kore 5, ko_Jamo ko_Kore 5, ko_Jamo ko_Hang 5, kvb ms 10,
    kvr ms 10, kxd ms 10, la it 20, lb de 4, lce ms 10, lcf ms 10, liw ms 10, lkb luy 10,
    lko luy 10, lks luy 10, lo_Laoo en_Latn 10, lri luy 10, lrm luy 10, lsm luy 10, ltg lv 10,
    lto luy 10, lts luy 10, lwg luy 10, lzh zh 10, mai hi 20, max ms 10, meo ms 10, mfa ms 10,
    mfb ms 10, mi en 20, min ms 10, ml_Latn ml_Mlym 20, ml_Mlym en_Latn 10, mnp zh 10,
    mqg ms 10, mr_Latn mr_Deva 20, msh mg 10, msi ms 10, mui ms 10, mvf mn 10,
    my_Mymr en_Latn 10, nan zh 10, nb no 1, nb da 8, nb nn 20, ne_Deva en_Latn 10, nhd gn 10,
    niq kln 10, nle luy 10, nn nb 20, nn no 20, no nb 1, no da 8, no nn 20, nyd luy 10,
    oc fr 20, oki kln 10, or_Orya en_Latn 10, orc om 10, orn ms 10, ors ms 10,
    pa_Guru en_Latn 10, pbt ps 10, pcm en 20, pel ms 10, pga ar 10, pko kln 10,
    ps_Arab en_Latn 10, pse ms 10, pst ps 10, qub qu 10, qud qu 10, quf qu 10, qug qu 10,
    quh qu 10, quk qu 10, qul qu 10, qup qu 10, qur qu 10, qus qu 10, quw qu 10, qux qu 10,
    quy qu 10, qva qu 10, qvc qu 10, qve qu 10, qvh qu 10, qvi qu 10, qvj qu 10, qvl qu 10,
    qvm qu 10, qvn qu 10, qvo qu 10, qvp qu 10, qvs qu 10, qvw qu 10, qvz qu 10, qwa qu 10,
    qwc qu 10, qwh qu 10, qws qu 10, qxa qu 10, qxc qu 10, qxh qu 10, qxl qu 10, qxn qu 10,
    qxo qu 10, qxp qu 10, qxr qu 10, qxt qu 10, qxu qu 10, qxw qu 10, rag luy 10, rm de 20,
    sd_Arab en_Latn 10, sdc sc 10, sdn sc 10, sgc kln 10, sh bs 4, sh hr 4, sh sr 4, shu ar 10,
    si_Sinh en_Latn 10, sk cs 20, skg mg 10, spv or 10, sr bs 5, sr hr 5, sr sh 4,
    sr_Cyrl sr_Latn 5, sr_Latn sr_Cyrl 5, sro sc 10, ssh ar 10, ssy aa 4, su id 20,
    ta_Latn ta_Taml 20, ta_Taml en_Latn 10, tdx mg 10, te_Latn te_Telu 20, te_Telu en_Latn 10,
    tec kln 10, ti_Ethi en_Latn 10, tk_Latn ru_Cyrl 10, tkg mg 10, tmw ms 10, tuy kln 10,
    txy mg 10, ug zh 20, uk ru 20, ur_Arab en_Latn 10, urk ms 10, uz_Latn ru_Cyrl 10,
    uzs uz 10, vkk ms 10, vkt ms 10, vro et 10, wuu zh 10, xmm ms 10, xmv mg 10, xmw mg 10,
    yi_Hebr en_Latn 10, yih yi 10, yue zh 10, za zh 20, za_Latn zh_Hans 10, zh_Hani zh_Hans 20,
    zh_Hani zh_Hant 20, zh_Latn zh_Hans 20, zlm ms 10, zmi ms 10,
";

const MAGHREB: &[&str] = &["MA", "DZ", "TN", "LY", "MR", "EH"];
const US: &[&str] = &["AS", "GU", "MH", "MP", "PR", "UM", "US", "VI"];
const CNSAR: &[&str] = &["HK", "MO"];
const LATIN_AMERICA: &[&str] = &[
    "419", "013", "BZ", "CR", "SV", "GT", "HN", "MX", "NI", "PA", "005", "AR", "BO", "BR", "CL",
    "CO", "EC", "FK", "GF", "GY", "PY", "PE", "SR", "UY", "VE",
];
/// 美洲中 LATIN_AMERICA 以外的地區。
const OTHER_AMERICAS: &[&str] = &[
    "019", "029", "AI", "AG", "AW", "BS", "BB", "VG", "BQ", "KY", "CU", "CW", "DM", "DO", "GD",
    "GP", "HT", "JM", "MQ", "MS", "PR", "SX", "BL", "KN", "LC", "MF", "VC", "TT", "TC", "VI",
    "021", "BM", "CA", "GL", "PM", "US", "003",
];

struct Triple {
    language: String,
    script: String,
    region: String,
}

fn close_pair(desired: &str, supported: &str) -> Option<u32> {
    CLOSE_PAIRS.split(',').find_map(|entry| {
        let mut parts = entry.split_whitespace();
        (parts.next()? == desired && parts.next()? == supported)
            .then(|| parts.next()?.parse().ok())?
    })
}

fn parse(tag: &str) -> Result<Locale, ParseError> {
    let tag = tag.replace('_', "-");
    Locale::try_from_str(&tag).or_else(|error| {
        // ICU 不收 extlang（zh-yue）：屬於前面語言的升為主語言、其餘（zh-CHT）丟掉，同 langcodes；
        // ICU 沒有 extlang 登記資料，「屬於」以語言距離表判斷。
        let mut parts = tag.splitn(3, '-');
        let (Some(prefix), Some(ext)) = (parts.next(), parts.next()) else {
            return Err(error);
        };
        if ext.len() != 3 || !ext.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(error);
        }
        let rest = parts.next().map(|r| format!("-{r}")).unwrap_or_default();
        let language = |code: &str| {
            let mut locale = Locale::try_from_str(code).ok()?;
            LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
            Some(locale.id.language.to_string())
        };
        let belongs = match (language(prefix), language(ext)) {
            (Some(p), Some(e)) => e == p || close_pair(&e, &p).is_some(),
            _ => false,
        };
        let head = if belongs { ext } else { prefix };
        Locale::try_from_str(&format!("{head}{rest}")).map_err(|_| error)
    })
}

fn maximize(tag: &str) -> Result<Triple, ParseError> {
    let mut locale = parse(tag)?;
    LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    let given_region = locale.id.region;
    LocaleExpander::new_extended().maximize(&mut locale.id);
    // ICU 把未知地區 ZZ 換成可能地區，langcodes 保留使用者給的地區。
    let region = given_region.or(locale.id.region);
    Ok(Triple {
        language: locale.id.language.to_string(),
        script: locale.id.script.map(|s| s.to_string()).unwrap_or_default(),
        region: region.map(|r| r.to_string()).unwrap_or_default(),
    })
}

fn territory_distance(desired: &Triple, supported: &Triple) -> u32 {
    let (d, s) = (desired.region.as_str(), supported.region.as_str());
    let americas = |r: &str| LATIN_AMERICA.contains(&r) || OTHER_AMERICAS.contains(&r);
    let differs = |set: &[&str]| set.contains(&d) != set.contains(&s);
    // 依語言分支互斥，同 langcodes 的 elif 鏈：ar、en 不套 419 規則。
    match desired.language.as_str() {
        "ar" if differs(MAGHREB) => 5,
        "ar" => DEFAULT_TERRITORY_DISTANCE,
        "en" if d == "GB" && !US.contains(&s) || !US.contains(&d) && s == "GB" => 3,
        "en" if differs(US) => 5,
        "en" => DEFAULT_TERRITORY_DISTANCE,
        _ if LATIN_AMERICA.contains(&d) && s == "419" => 1,
        "es" | "pt" if americas(d) != americas(s) => 5,
        "es" | "pt" => DEFAULT_TERRITORY_DISTANCE,
        "zh" if desired.script == "Hant" && differs(CNSAR) => 5,
        _ => DEFAULT_TERRITORY_DISTANCE,
    }
}

fn distance(desired: &Triple, supported: &Triple) -> u32 {
    let mut distance = 0;
    if desired.language != supported.language {
        distance +=
            close_pair(&desired.language, &supported.language).unwrap_or(DEFAULT_LANGUAGE_DISTANCE);
    }
    let desired_pair = format!("{}_{}", desired.language, desired.script);
    let supported_pair = format!("{}_{}", supported.language, supported.script);
    if desired.script != supported.script {
        distance += close_pair(&desired_pair, &supported_pair).unwrap_or(DEFAULT_SCRIPT_DISTANCE);
    }
    if desired.region != supported.region {
        distance += if desired_pair == supported_pair {
            territory_distance(desired, supported)
        } else {
            DEFAULT_TERRITORY_DISTANCE
        };
    }
    distance
}

/// 語言檔的地區是否為該語言（＋字體系）的可能地區，例如 fr-FR、pt-BR、zh-Hant-TW。
fn is_default_region(catalog: &Triple) -> bool {
    let base = if catalog.script.is_empty() {
        catalog.language.clone()
    } else {
        format!("{}-{}", catalog.language, catalog.script)
    };
    maximize(&base).is_ok_and(|t| t.region == catalog.region)
}

/// 語言標籤 → `offered` 中距離最近的語言碼與距離，都太遠回 `None`；標籤解析失敗回錯誤。
///
/// 同距離時取語言預設地區的語言檔（fr-BE 在 fr-CA、fr-FR 之間取 fr-FR），都不是才取先出現者；
/// Python 版只取先出現者，這裡刻意不同。
pub fn closest_match<'a>(
    tag: &str,
    offered: &'a [String],
) -> Result<Option<(&'a str, u32)>, ParseError> {
    if let Some(code) = offered.iter().find(|code| *code == tag) {
        return Ok(Some((code, 0)));
    }
    let desired = maximize(tag)?;
    let candidates: Vec<(&str, u32, Triple)> = offered
        .iter()
        .filter_map(|code| {
            let supported = maximize(code).ok()?;
            let d = distance(&desired, &supported);
            (d <= MAX_DISTANCE).then_some((code.as_str(), d, supported))
        })
        .collect();
    let Some(best) = candidates.iter().map(|(_, d, _)| *d).min() else {
        return Ok(None);
    };
    let ties: Vec<_> = candidates.iter().filter(|(_, d, _)| *d == best).collect();
    let chosen = ties
        .iter()
        .find(|(_, _, supported)| is_default_region(supported))
        .unwrap_or(&ties[0]);
    Ok(Some((chosen.0, best)))
}
