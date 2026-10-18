# Python 測試對照稽核

目的：Python 版將在第 5 段刪除，這份文件逐一列出 `tests/` 底下每個 Python 測試函式在 Rust 版的對應，確保行為沒有因為移除 Python 而失去測試保護。每個 Python 檔一節，每列一個測試函式（參數化測試也只算一列）：「Python 測試 → Rust 測試」；一個 Python 測試可以對應多個 Rust 測試（以逗號分隔），已由 golden fixture 覆蓋的對應到消費該 fixture 的 golden 測試；沒有對應者標「不移植：理由」，理由限於 Tk 專屬、PyInstaller／啟動畫面專屬、Python 套件專屬，以及 Rust 設計已依規格刻意不同或 Python 才有的機制。Rust 位置一律以 crate 為根（`tests/x.rs::fn` 為整合測試，`src/a/b.rs::fn` 為該檔 `mod tests` 內的單元測試），不加 `rust/` 前綴，因為 `rust/` 會在第 5 段搬到 repo 根目錄。

## tests/composer/test_paste.py

- `test_typing_never_re_presses_the_modifiers_the_user_held` → `src/composer/paste.rs::typing_never_re_presses_the_modifiers_the_user_held`
- `test_type_into_window_types_text_one_char_at_a_time` → `src/composer/paste.rs::type_into_window_types_text_one_char_at_a_time`
- `test_type_into_window_flattens_newlines` → `src/composer/paste.rs::type_into_window_flattens_newlines`
- `test_type_into_window_stops_when_foreground_leaves_target` → `src/composer/paste.rs::type_into_window_stops_when_foreground_leaves_target`
- `test_type_into_window_never_sends_enter` → `src/composer/paste.rs::type_into_window_never_sends_enter`
- `test_foreground_exe_returns_none_when_lookup_fails` → `src/composer/paste.rs::foreground_exe_returns_none_when_lookup_fails`
- `test_interceptor_passes_plain_v_through` → `src/composer/paste.rs::interceptor_passes_plain_v_through`
- `test_interceptor_passes_ctrl_v_through_outside_the_game` → `src/composer/paste.rs::interceptor_passes_ctrl_v_through_outside_the_game`
- `test_interceptor_swallows_ctrl_v_in_the_game_and_pastes_once` → `src/composer/paste.rs::interceptor_swallows_ctrl_v_in_the_game_and_pastes_once`
- `test_interceptor_ignores_other_keys` → `src/composer/paste.rs::interceptor_ignores_other_keys`
- `test_interceptor_ignores_key_repeat_until_released` → `src/composer/paste.rs::interceptor_ignores_key_repeat_until_released`
- `test_interceptor_keeps_swallowing_v_until_released_even_after_ctrl_is_up` → `src/composer/paste.rs::interceptor_keeps_swallowing_v_until_released_even_after_ctrl_is_up`
- `test_clipboard_text_reads_unicode_text` → `src/composer/paste.rs::paste_clipboard_reads_unicode_text`
- `test_clipboard_text_returns_none_when_clipboard_has_no_text` → `src/composer/paste.rs::paste_clipboard_skips_unreadable_clipboard`
- `test_paste_clipboard_types_the_clipboard_with_the_configured_delay` → `src/composer/paste.rs::paste_clipboard_types_the_clipboard_with_the_configured_delay`
- `test_paste_clipboard_skips_empty_clipboard` → `src/composer/paste.rs::paste_clipboard_skips_empty_clipboard`
- `test_paste_clipboard_types_the_text_verbatim_like_native_paste` → `src/composer/paste.rs::paste_clipboard_types_the_text_verbatim_like_native_paste`
- `test_paste_clipboard_turns_line_breaks_into_spaces_for_the_chat_box` → `src/composer/paste.rs::paste_clipboard_turns_line_breaks_into_spaces_for_the_chat_box`
- `test_paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing` → `src/composer/paste.rs::paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing`
- `test_paste_clipboard_gives_up_waiting_after_the_timeout` → `src/composer/paste.rs::paste_clipboard_gives_up_waiting_after_the_timeout`
- `test_paste_clipboard_stops_when_foreground_leaves_target` → `src/composer/paste.rs::paste_clipboard_stops_when_foreground_leaves_target`
- `test_paste_clipboard_ignores_a_second_paste_while_typing` → `src/composer/paste.rs::paste_clipboard_ignores_a_second_paste_while_typing`
- `test_interceptor_swallows_v_while_typing_is_in_progress` → `src/composer/paste.rs::interceptor_swallows_v_while_typing_is_in_progress`
- `test_typing_in_progress_is_true_only_while_chars_are_being_typed` → `src/composer/paste.rs::typing_in_progress_is_true_only_while_chars_are_being_typed`

## tests/config/test_config.py

- `test_load_missing_file_returns_defaults` → `tests/config_golden.rs::load_missing_file_returns_defaults`
- `test_load_merges_partial_file_with_defaults` → `tests/config_golden.rs::load_matches_python`
- `test_removed_poll_interval_is_dropped_from_the_file` → `tests/config_golden.rs::load_matches_python`
- `test_save_then_load_roundtrip` → `tests/config_golden.rs::save_then_load_roundtrip`, `tests/config_golden.rs::save_bytes_match_python`
- `test_app_dir_dev_mode_is_project_root` → `src/config/mod.rs::app_dir_in_debug_is_the_project_root`
- `test_app_dir_frozen_uses_executable_dir` → 不移植：PyInstaller 的 `sys.frozen`／`sys.executable` 機制；Rust 以 `cfg!(debug_assertions)` 區分開發與發行版，沒有可注入的執行期旗標
- `test_legacy_api_block_is_migrated_and_rewritten` → `tests/config_golden.rs::load_matches_python`
- `test_an_unsupported_service_survives_the_rewrite_and_a_roundtrip` → `tests/config_golden.rs::load_matches_python`, `tests/config_golden.rs::save_bytes_match_python`
- `test_load_does_not_rewrite_a_current_file` → `tests/config_golden.rs::load_matches_python`
- `test_load_survives_a_read_only_config_file` → `tests/config_golden.rs::read_only_file_still_loads_after_migration`
- `test_load_clamps_out_of_range_advanced_values` → `tests/config_golden.rs::load_matches_python`
- `test_parallel_translations_defaults_and_clamps` → `src/config/mod.rs::clamp_keeps_python_int_and_float_types`, `tests/config_services.rs::parse_advanced_clamps_parallel`, `tests/config_golden.rs::load_matches_python`
- `test_is_configured_needs_a_complete_default_service` → `tests/config_golden.rs::is_configured_needs_a_complete_default_service`
- `test_default_config_has_ui_language` → `src/config/mod.rs::default_region_hotkey_differs_from_the_input_hotkey`
- `test_app_name_follows_language` → `tests/i18n_golden.rs::app_name_follows_language`
- `test_old_config_without_ui_language_loads` → `tests/config_golden.rs::load_matches_python`
- `test_old_config_with_leftover_game_path_loads_without_error` → `tests/config_golden.rs::load_matches_python`
- `test_local_state_dir_uses_localappdata` → `src/config/mod.rs::local_state_dir_is_under_localappdata`
- `test_local_state_dir_falls_back_to_home_without_localappdata` → `src/config/mod.rs::local_state_dir_falls_back_to_home_without_localappdata`
- `test_hook_state_shares_the_same_state_dir` → 不移植：Python 才有 `hook_state.STATE_DIR` 這個模組常數；Rust 的 `hook_state` 由呼叫端傳入目錄，`mem_reader` 預設值就是 `local_state_dir()`，沒有第二個來源可分岐
- `test_translate_system_messages_defaults_to_off` → `src/config/mod.rs::default_region_hotkey_differs_from_the_input_hotkey`
- `test_load_config_fills_in_translate_system_messages` → `tests/config_golden.rs::load_matches_python`
- `test_load_config_with_broken_json_falls_back_to_defaults` → `tests/config_golden.rs::load_config_with_broken_json_falls_back_to_defaults`, `tests/config_golden.rs::corrupt_file_is_never_overwritten`
- `test_load_config_with_non_numeric_advanced_value_uses_the_default` → `tests/config_golden.rs::load_config_with_non_numeric_advanced_value_uses_the_default`
- `test_default_config_has_a_region_hotkey_distinct_from_the_input_hotkey` → `src/config/mod.rs::default_region_hotkey_differs_from_the_input_hotkey`
- `test_config_summary_lists_every_slot_and_never_leaks_the_key` → `tests/config_golden.rs::summary_matches_python`

## tests/config/test_services.py

- `test_configured_cfg_passes_the_completeness_check` → 不移植：測試輔助函式 `configured_cfg` 本身的測試；Rust 測試各自組設定，沒有共用的同名輔助函式
- `test_configured_cfg_returns_an_independent_copy` → 不移植：測試輔助函式 `configured_cfg` 本身的測試；Rust 測試各自組設定，沒有共用的同名輔助函式
- `test_every_provider_has_ui_metadata` → `tests/config_services.rs::every_provider_has_ui_metadata`
- `test_short_names_are_brands_except_the_custom_endpoint` → `tests/config_services.rs::short_names_are_brands_except_the_custom_endpoint`
- `test_validate_service_reports_every_missing_required_field` → `tests/config_services.rs::validate_service_reports_every_missing_required_field`
- `test_validate_endpoint_fields_ignores_the_model` → `tests/config_services.rs::validate_endpoint_fields_ignores_the_model`
- `test_profile_fields_cover_what_the_translator_takes` → `tests/config_services.rs::profile_fields_cover_what_the_translator_takes`
- `test_new_service_fills_the_provider_defaults_and_a_name` → `tests/config_services.rs::new_service_fills_the_provider_defaults_and_a_name`
- `test_new_services_never_share_an_id` → `tests/config_services.rs::new_services_never_share_an_id`
- `test_unique_name_numbers_collisions_from_two` → `tests/config_services.rs::unique_name_numbers_collisions_from_two`
- `test_unique_name_does_not_collide_with_the_entry_being_edited` → `tests/config_services.rs::unique_name_does_not_collide_with_the_entry_being_edited`
- `test_is_auto_name_accepts_the_bare_base_and_numbered_suffixes` → `tests/config_services.rs::is_auto_name_accepts_the_bare_base_and_numbered_suffixes`
- `test_is_auto_name_rejects_shapes_unique_name_never_produces` → `tests/config_services.rs::is_auto_name_rejects_shapes_unique_name_never_produces`
- `test_is_auto_name_matches_what_unique_name_actually_generates` → `tests/config_services.rs::is_auto_name_accepts_exactly_the_numbers_unique_name_can_produce`, `tests/config_golden.rs::is_auto_name_matches_python`
- `test_is_auto_name_accepts_exactly_the_numbers_unique_name_can_produce` → `tests/config_services.rs::is_auto_name_accepts_exactly_the_numbers_unique_name_can_produce`
- `test_resolve_follows_the_default_when_a_slot_is_unset` → `tests/config_golden.rs::resolve_matches_python`
- `test_resolve_prefers_the_slots_own_service` → `tests/config_golden.rs::resolve_matches_python`
- `test_resolve_drops_the_bookkeeping_fields` → `tests/config_golden.rs::resolve_matches_python`
- `test_resolve_returns_a_copy` → 不移植：Rust 的 `resolve` 回傳擁有所有權的 `ServiceSpec`，無法經由它改動原設定；Python 才需要防範別名
- `test_find_returns_none_for_an_unknown_id` → `tests/config_services.rs::find_returns_none_for_an_unknown_id`
- `test_describe_shows_the_brand_and_the_model` → `tests/config_services.rs::describe_shows_the_brand_and_the_model`
- `test_migrates_the_flat_legacy_api_block` → `tests/config_golden.rs::normalize_matches_python`
- `test_migrates_a_legacy_block_without_a_provider_as_a_custom_endpoint` → `tests/config_golden.rs::normalize_matches_python`
- `test_migrates_the_per_provider_api_block_keeping_every_filled_provider` → `tests/config_golden.rs::normalize_matches_python`
- `test_migration_of_an_untouched_config_leaves_an_empty_list` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_leftover_api_block_does_not_wipe_an_existing_service_list` → `tests/config_golden.rs::normalize_matches_python`
- `test_quarantines_a_service_with_an_unknown_provider` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_quarantined_service_is_never_pointed_at` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_hand_edited_quarantine_key_keeps_what_was_already_there` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_quarantined_service_returns_once_its_provider_is_known` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_api_block_survives_a_service_list_that_is_wholly_quarantined` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_filled_api_block_still_loses_to_a_service_list_that_survives` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_service_list_hand_edited_into_a_string_lands_in_quarantine` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_service_list_hand_edited_into_a_dict_lands_in_quarantine` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_non_list_service_list_survives_a_promotion_rewriting_it` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_service_list_hand_edited_into_one_bare_service_is_wrapped` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_bare_service_dict_this_build_cannot_read_lands_in_quarantine` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_api_block_survives_alongside_a_promoted_service` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_promoted_service_without_a_name_does_not_break_the_migration` → `tests/config_golden.rs::normalize_matches_python`
- `test_the_default_follows_the_migrated_service_when_the_api_provider_is_junk` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_unknown_provider_in_the_api_block_lands_in_quarantine` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_unknown_provider_in_the_flat_legacy_api_block_lands_in_quarantine` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_known_api_section_migrates_while_an_unknown_one_is_quarantined` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_leftover_api_block_still_rescues_its_unknown_sections` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_empty_service_is_not_evidence_that_the_api_block_was_migrated` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_service_filled_only_with_junk_types_is_not_evidence_either` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_config_without_a_service_list_reports_no_change` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_key_pasted_over_the_service_list_survives_an_api_migration` → `tests/config_golden.rs::normalize_matches_python`
- `test_the_service_list_steps_stand_on_their_own_without_a_container_pass` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_field_whose_default_is_none_keeps_the_real_value` → 不移植：Python 以預設值型別比對欄位型別，預設值為 None 時會誤清真值；Rust 的欄位預設是型別化的 `Field::Text`／`Field::Flag`（`src/config/services.rs`），沒有「預設值為 None」這種情形
- `test_fills_missing_fields_and_drops_stale_ones` → `tests/config_golden.rs::normalize_matches_python`
- `test_regenerates_duplicate_ids` → `tests/config_golden.rs::normalize_matches_python`
- `test_default_service_falls_back_to_the_first_entry` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_slot_pointing_at_nothing_falls_back_to_the_default` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_tidy_config_reports_no_change` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_null_field_in_a_migrated_api_block_stays_usable` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_null_field_in_the_flat_legacy_block_keeps_the_key` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_field_of_the_wrong_type_falls_back_to_the_provider_default` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_provider_of_the_wrong_type_drops_the_entry` → `tests/config_golden.rs::normalize_matches_python`
- `test_an_id_or_name_of_the_wrong_type_is_regenerated` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_default_service_of_the_wrong_type_falls_back_to_the_first_entry` → `tests/config_golden.rs::normalize_matches_python`
- `test_a_slot_of_the_wrong_type_falls_back_to_the_default` → `tests/config_golden.rs::normalize_matches_python`
- `test_two_services_sharing_a_name_load_with_distinct_names` → `tests/config_golden.rs::normalize_matches_python`
- `test_every_provider_label_and_description_resolve` → `tests/config_services.rs::every_provider_label_and_description_resolve`

## tests/i18n/test_i18n.py

- `test_no_language_file_has_keys_the_source_language_lacks` → `src/i18n/mod.rs::no_language_file_has_keys_the_source_language_lacks`
- `test_the_hand_maintained_catalogs_carry_the_same_keys` → `src/i18n/mod.rs::the_hand_maintained_catalogs_carry_the_same_keys`
- `test_placeholders_match_across_languages` → `src/i18n/mod.rs::placeholders_match_across_languages`
- `test_language_files_have_no_duplicate_keys` → `src/i18n/mod.rs::every_embedded_catalog_parses_without_duplicates`, `tests/i18n_golden.rs::duplicate_keys_are_rejected`
- `test_t_returns_current_language_string` → `tests/i18n_golden.rs::t_matches_python`, `tests/i18n_golden.rs::app_name_follows_language`
- `test_t_formats_named_placeholders` → `src/i18n/mod.rs::t_formats_named_placeholders`
- `test_fallback_order_is_current_then_english_then_source` → `src/i18n/mod.rs::fallback_order_is_current_then_english_then_source`
- `test_missing_key_falls_back_to_english_before_source` → `src/i18n/mod.rs::missing_key_falls_back_to_english_before_source`
- `test_missing_key_falls_back_to_source_language_last` → `src/i18n/mod.rs::missing_key_falls_back_to_source_language_last`
- `test_missing_everywhere_returns_the_key_itself` → `src/i18n/mod.rs::missing_everywhere_returns_the_key_itself`
- `test_broken_placeholder_falls_back_to_the_next_language` → `src/i18n/mod.rs::broken_placeholder_falls_back_to_the_next_language`
- `test_unknown_language_code_falls_back_to_default` → `tests/i18n_golden.rs::unknown_language_code_falls_back_to_default`
- `test_best_match_picks_the_closest_catalog` → `tests/i18n_golden.rs::best_match_matches_python`
- `test_best_match_follows_the_preference_order` → `tests/i18n_golden.rs::best_match_matches_python`
- `test_best_match_falls_back_to_default_when_nothing_is_close` → `tests/i18n_golden.rs::best_match_matches_python`
- `test_system_preferences_are_valid_language_tags` → `src/i18n/mod.rs::system_preferences_are_valid_language_tags`, `tests/i18n_golden.rs::system_language_detection_returns_an_offered_code`
- `test_languages_are_listed_as_endonyms` → `tests/i18n_golden.rs::languages_are_listed_as_endonyms`, `tests/i18n_golden.rs::available_languages_match_python`
- `test_language_names_are_unique` → `tests/i18n_golden.rs::language_names_are_unique`
- `test_a_new_language_file_needs_no_code_change` → 不移植：Rust 的語言檔以 `include_str!` 編進 exe（規格「單一 exe 與資源」節），新增語言要在 `CATALOG_SOURCES` 加一行，也沒有執行期替換語言檔目錄的機制；「選單只看語言檔內容」的部分由 `offered_languages` 的三則測試覆蓋
- `test_same_language_different_scripts_are_told_apart` → `tests/i18n_golden.rs::best_match_matches_python`
- `test_regional_variants_are_told_apart` → 不移植：已知缺口，不是規格豁免：Rust 的 `best_match` 對同語言的多個語言檔取第一個，Python 以 CLDR 取地區最接近者（pt-MZ 在 Python 得 pt-PT、在 Rust 得 pt-BR）；目前每個語言只有一份語言檔（en-US、zh-CN、zh-TW），所以不影響結果，後續處理見 `2026-10-17-rust-rewrite-followups.md` 的「第 5 段」
- `test_a_language_without_an_endonym_is_not_offered` → `src/i18n/mod.rs::a_language_without_an_endonym_is_not_offered`
- `test_a_barely_translated_language_is_not_offered` → `src/i18n/mod.rs::a_barely_translated_language_is_not_offered`
- `test_the_source_and_default_languages_are_always_offered` → `src/i18n/mod.rs::the_source_and_default_languages_are_always_offered`
- `test_translators_are_never_borrowed_from_another_language` → `src/i18n/mod.rs::translators_are_never_borrowed_from_another_language`
- `test_translators_is_optional_metadata` → `src/i18n/mod.rs::translators_are_never_borrowed_from_another_language`
- `test_unknown_language_code_is_rejected_by_the_scanned_list` → `tests/i18n_golden.rs::unknown_language_code_falls_back_to_default`
- `test_set_language_keeps_previous_language_when_load_fails` → 不移植：Rust 的語言檔在程式啟動時解析一次（`OnceLock`），`set_language` 只換語言碼、沒有逐次載入，不存在「載入失敗」的路徑
- `test_zh_cn_keeps_its_own_wording_for_chat_and_advanced` → `src/i18n/mod.rs::zh_cn_keeps_its_own_wording_for_chat_and_advanced`
- `test_the_zh_cn_readme_speaks_the_same_words_as_the_catalog` → `src/i18n/mod.rs::the_zh_cn_readme_speaks_the_same_words_as_the_catalog`

## tests/input/test_dispatch.py

- `test_open_in_the_foreground_window_anchors_and_shows` → `src/input/dispatch.rs::open_in_the_foreground_window_anchors_and_shows`
- `test_open_in_the_foreground_window_retargets_before_showing` → `src/input/dispatch.rs::open_in_the_foreground_window_retargets_before_showing`
- `test_open_in_a_background_window_is_tracked_but_not_shown` → `src/input/dispatch.rs::open_in_a_background_window_is_tracked_but_not_shown`
- `test_close_from_another_window_does_not_hide_the_box` → `src/input/dispatch.rs::close_from_another_window_does_not_hide_the_box`
- `test_auto_show_off_still_tracks_and_anchors` → `src/input/dispatch.rs::auto_show_off_still_tracks_and_anchors`
- `test_paste_single_line_follows_the_foreground_window` → `src/input/dispatch.rs::paste_single_line_follows_the_foreground_window`
- `test_on_hotkey_retargets_before_showing_when_game_is_foreground` → `src/input/dispatch.rs::on_hotkey_retargets_before_showing_when_game_is_foreground`
- `test_on_hotkey_only_refocuses_when_the_box_itself_is_foreground` → `src/input/dispatch.rs::on_hotkey_only_refocuses_when_the_box_itself_is_foreground`
- `test_on_hotkey_ignored_when_box_closed_and_game_not_foreground` → `src/input/dispatch.rs::on_hotkey_ignored_when_box_closed_and_game_not_foreground`
- `test_on_hotkey_does_not_log_ignored_when_box_open_and_foreground_is_not_the_game` → `tests/dispatch.rs::on_hotkey_does_not_log_ignored_when_box_open_and_foreground_is_not_the_game`
- `test_hotkey_opens_input_box_when_game_is_foreground` → `src/input/dispatch.rs::hotkey_opens_input_box_when_game_is_foreground`
- `test_hotkey_ignored_when_other_window_is_foreground` → `src/input/dispatch.rs::hotkey_ignored_when_other_window_is_foreground`
- `test_hotkey_ignored_when_foreground_unknown` → `src/input/dispatch.rs::hotkey_ignored_when_foreground_unknown`
- `test_hotkey_refocuses_open_input_box_regardless_of_foreground` → `src/input/dispatch.rs::hotkey_refocuses_open_input_box_regardless_of_foreground`
- `test_region_hotkey_to_register_returns_none_on_collision` → `tests/dispatch.rs::region_hotkey_to_register_returns_none_on_collision`
- `test_region_hotkey_to_register_returns_the_hotkey_when_different` → `src/input/dispatch.rs::region_hotkey_to_register_returns_the_hotkey_when_different`
- `test_region_hotkey_cancels_when_the_selector_is_already_open` → `src/input/dispatch.rs::region_hotkey_cancels_when_the_selector_is_already_open`
- `test_region_hotkey_starts_a_selection_when_game_is_foreground` → `src/input/dispatch.rs::region_hotkey_starts_a_selection_when_game_is_foreground`
- `test_region_hotkey_ignored_when_other_window_is_foreground` → `src/input/dispatch.rs::region_hotkey_ignored_when_other_window_is_foreground`
- `test_paste_intercepted_only_when_enabled_and_game_is_foreground` → `src/input/dispatch.rs::paste_intercepted_only_when_enabled_and_game_is_foreground`
- `test_paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open` → `tests/app_main.rs::paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open`

## tests/input/test_hotkeys.py

- `test_swallows_the_trigger_key_when_the_combo_is_handled` → `src/input/hotkeys.rs::swallows_the_trigger_key_when_the_combo_is_handled`
- `test_passes_the_key_through_when_the_callback_does_not_handle_it` → `src/input/hotkeys.rs::passes_the_key_through_when_the_callback_does_not_handle_it`
- `test_masks_modifiers_only_when_swallowing` → `src/input/hotkeys.rs::masks_modifiers_only_when_swallowing`
- `test_passes_the_bare_trigger_key_through` → `src/input/hotkeys.rs::passes_the_bare_trigger_key_through`
- `test_extra_modifier_does_not_match` → `src/input/hotkeys.rs::extra_modifier_does_not_match`
- `test_extra_non_modifier_key_still_matches` → `src/input/hotkeys.rs::extra_non_modifier_key_still_matches`
- `test_unrelated_keys_pass_without_calling_back` → `src/input/hotkeys.rs::unrelated_keys_pass_without_calling_back`
- `test_key_repeat_is_swallowed_until_release_even_after_the_modifier_is_up` → `src/input/hotkeys.rs::key_repeat_is_swallowed_until_release_even_after_the_modifier_is_up`
- `test_combo_without_modifiers_swallows_whichever_key_completes_it` → `src/input/hotkeys.rs::combo_without_modifiers_swallows_whichever_key_completes_it`
- `test_rejects_combos_it_cannot_swallow` → `src/input/hotkeys.rs::rejects_combos_it_cannot_swallow`
- `test_register_hotkey_installs_a_suppressing_hook` → `tests/hotkeys.rs::register_hotkey_installs_a_swallowing_handler`
- `test_register_hotkey_falls_back_to_add_hotkey_when_it_cannot_swallow` → `tests/hotkeys.rs::register_hotkey_falls_back_to_passive_when_it_cannot_swallow`
- `test_register_hotkey_falls_back_to_the_default_on_an_unknown_key` → `tests/hotkeys.rs::register_hotkey_falls_back_to_the_default_on_an_unknown_key`
- `test_register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key` → `tests/hotkeys.rs::register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key`

## tests/logs/test_files.py

- `test_utc_stamp_has_millisecond_precision` → `tests/logs_golden.rs::stamp_matches_python`
- `test_utc_stamp_converts_to_utc` → `tests/logs_golden.rs::stamp_matches_python`
- `test_each_line_gets_a_stamp_prefix` → `src/logs/files.rs::each_line_gets_a_stamp_prefix`
- `test_split_writes_share_one_line_prefix` → `src/logs/files.rs::writer_stamps_each_line_once_across_split_writes`
- `test_next_line_after_split_write_gets_new_stamp` → `src/logs/files.rs::next_line_after_split_write_gets_new_stamp`
- `test_blank_line_is_stamped` → `src/logs/files.rs::writer_stamps_each_line_once_across_split_writes`
- `test_empty_write_emits_nothing` → `src/logs/files.rs::empty_write_emits_nothing`
- `test_flush_and_close_reach_the_wrapped_stream` → `src/logs/files.rs::flush_reaches_the_wrapped_writer`
- `test_recent_session_kept` → `tests/logs_golden.rs::trim_matches_python`
- `test_expired_session_dropped` → `tests/logs_golden.rs::trim_matches_python`
- `test_mixed_sessions_keep_only_recent` → `tests/logs_golden.rs::trim_matches_python`
- `test_headerless_legacy_content_dropped` → `tests/logs_golden.rs::trim_matches_python`
- `test_unparseable_header_section_dropped` → `tests/logs_golden.rs::trim_matches_python`
- `test_empty_text_stays_empty` → `tests/logs_golden.rs::trim_matches_python`
- `test_concurrent_writers_keep_every_line_stamped` → `src/logs/mod.rs::log_from_many_threads_keeps_every_line_whole`
- `test_log_writes_a_whole_line_in_one_call` → `src/logs/mod.rs::log_from_many_threads_keeps_every_line_whole`, `src/logs/mod.rs::multiline_message_stamps_and_crlfs_every_line`
- `test_prepare_log_rewrite_keeps_single_crlf` → `src/logs/files.rs::lines_are_crlf`
- `test_prepare_log_repairs_stacked_carriage_returns` → `src/logs/files.rs::stacked_carriage_returns_are_repaired`
- `test_prepare_log_leaves_healthy_file_untouched` → `src/logs/files.rs::healthy_file_is_left_untouched`

## tests/region/test_capture.py

- `test_rect_inside_the_client_is_shifted_to_client_origin` → `tests/capture_golden.rs::window_region_matches_python`
- `test_rect_overlapping_the_edges_is_clipped` → `tests/capture_golden.rs::window_region_matches_python`
- `test_rect_entirely_outside_the_client_is_none` → `tests/capture_golden.rs::window_region_matches_python`
- `test_rect_touching_the_edge_without_overlap_is_none` → `tests/capture_golden.rs::window_region_matches_python`
- `test_selection_outside_game_is_a_capture_error` → `tests/capture.rs::crop_frame_outside_the_game_is_outside_game`
- `test_non_capture_error_during_capture_becomes_a_capture_error` → `tests/capture.rs::capture_failures_are_wrapped_as_failed`, `tests/capture.rs::capture_window_of_a_missing_window_fails_before_wgc`
- `test_capture_window_crops_the_client_area_out_of_the_dwm_frame` → `tests/capture.rs::capture_offsets_the_client_area_inside_the_dwm_frame`
- `test_single_colour_capture_is_a_capture_error` → `tests/capture.rs::blank_capture_is_an_error`
- `test_grab_window_converts_the_first_frame_from_bgra` → `src/region/capture.rs::grab_window_converts_the_first_frame_from_rgba`, `src/region/capture.rs::first_frame_is_returned`
- `test_grab_window_times_out_when_no_frame_arrives` → `src/region/capture.rs::grab_times_out_after_two_seconds`
- `test_grab_window_session_closed_before_a_frame_is_a_capture_error` → `src/region/capture.rs::session_closed_before_a_frame_is_an_error`
- `test_crop_frame_returns_a_png_matching_the_selected_rect` → `tests/capture.rs::crop_frame_cuts_the_selected_rect_out_of_the_frame`
- `test_crop_frame_clips_a_rect_partially_outside_the_client` → `tests/capture.rs::crop_frame_clips_to_the_client_area`
- `test_crop_frame_rect_entirely_outside_the_client_is_selection_outside_game` → `tests/capture.rs::crop_frame_outside_the_game_is_outside_game`
- `test_capture_screen_falls_back_to_black_when_grab_fails` → `tests/capture.rs::capture_screen_falls_back_to_black_when_grab_fails`
- `test_capture_screen_returns_the_grabbed_image_on_success` → `tests/capture.rs::capture_screen_returns_the_grabbed_image_on_success`

## tests/region/test_ocr.py

- `test_merge_lines_joins_boxes_on_one_visual_line` → `tests/ocr_golden.rs::merge_lines_matches_python`
- `test_merge_lines_orders_lines_top_to_bottom` → `tests/ocr_golden.rs::merge_lines_matches_python`
- `test_merge_lines_returns_empty_list_for_no_boxes` → `tests/ocr_golden.rs::merge_lines_matches_python`
- `test_recognize_reads_rendered_text` → `tests/ocr_golden.rs::recognize_matches_python`
- `test_recognize_reads_text_on_a_transparent_background` → `tests/ocr_golden.rs::recognize_matches_python`
- `test_recognize_returns_empty_for_a_blank_image` → `tests/ocr_golden.rs::recognize_matches_python`

## tests/region/test_pipeline.py

- `test_ocr_text_is_translated_and_source_is_kept` → `tests/region_pipeline.rs::run_returns_translation_and_source`
- `test_ocr_without_text_returns_an_empty_result_without_translating` → `tests/region_pipeline.rs::no_text_skips_translation`
- `test_ocr_unavailable_propagates` → `tests/region_pipeline.rs::errors_propagate`
- `test_translator_config_error_propagates` → `tests/region_pipeline.rs::errors_propagate`
- `test_translator_offline_error_propagates` → `tests/region_pipeline.rs::translator_offline_error_propagates`
- `test_the_cancel_handle_is_passed_to_the_translator` → `tests/region_pipeline.rs::cancel_is_passed_to_the_translator`

## tests/runtime/test_resources.py

- `test_bundle_dir_points_at_src_subdirectory` → 不移植：Python 的 `bundle_dir` 解析 PyInstaller 的資源目錄；Rust 以 `include_str!`／`include_bytes!` 把資源編進 exe（規格「單一 exe 與資源」節），沒有資源目錄
- `test_icon_file_exists` → `src/runtime/resources.rs::ico_has_expected_sizes`
- `test_icon_is_valid_ico_with_expected_sizes` → `src/runtime/resources.rs::ico_has_expected_sizes`
- `test_png_icons_exist_at_the_sizes_tkinter_needs` → `src/runtime/resources.rs::icon_pngs_have_native_sizes`
- `test_build_spec_bundles_and_applies_the_icon` → 不移植：PyInstaller 專屬（`build.spec` 的圖示與資料收錄）；Rust 版圖示由 `build.rs` 編進 exe，並由 `tests/app_main.rs::exe_embeds_the_app_icon_resource` 覆蓋
- `test_build_spec_wires_up_the_splash_screen` → 不移植：PyInstaller 啟動畫面專屬（`build.spec`）
- `test_build_spec_installs_the_progress_bar_before_constructing_splash` → 不移植：PyInstaller 啟動畫面專屬（`build.spec`）

## tests/runtime/test_single_instance.py

- `test_first_instance_acquires_the_mutex` → `src/runtime/single_instance.rs::released_on_drop`
- `test_second_instance_is_refused` → `src/runtime/single_instance.rs::second_acquire_fails_while_first_held`
- `test_focus_reports_failure_when_no_window_matches` → `src/runtime/single_instance.rs::focus_reports_failure_when_no_window_matches`
- `test_pid_from_mei_name_parses_hex_pid` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_pid_from_mei_name_rejects_non_hex_suffix` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_pid_from_mei_name_rejects_non_mei_prefix` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_is_sibling_excludes_self` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_is_sibling_excludes_parent_bootloader` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_is_sibling_matches_path_case_insensitively` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_is_sibling_false_when_exe_path_lookup_fails` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_is_sibling_false_when_path_differs` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_start_instance_watch_returns_none_when_not_frozen` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_start_instance_watch_disables_on_self_check_mismatch` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_watch_iteration_preempts_sibling_and_cleans_up` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄
- `test_watch_iteration_ignores_non_sibling_directory` → 不移植：PyInstaller 專屬（以 `_MEI` 暫存目錄辨識並攔截兄弟 bootloader）；Rust 單一 exe 沒有解壓暫存目錄

## tests/runtime/test_splash.py

- `test_phase_constants_match_the_literals_run_py_and_main_py_used_to_pass` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_no_ops_without_pyi_splash` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_forwards_to_pyi_splash` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_update_after_close_is_ignored` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_close_is_idempotent` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_failures_are_swallowed` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_drain_logs_flushes_buffer_then_switches_to_direct_logging` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）

## tests/runtime/test_updater.py

- `test_parse_version_accepts_plain_and_v_prefixed` → `tests/updater.rs::parse_version_accepts_plain_and_v_prefixed`
- `test_parse_version_ignores_suffixes` → `tests/updater.rs::parse_version_ignores_suffixes`
- `test_parse_version_rejects_unparsable_text` → `tests/updater.rs::parse_version_rejects_unparsable_text`
- `test_is_newer_compares_numerically` → `tests/updater.rs::is_newer_compares_numerically`
- `test_is_newer_ranks_a_release_above_its_own_prereleases` → `tests/updater.rs::is_newer_ranks_a_release_above_its_own_prereleases`
- `test_is_newer_compares_numeric_prerelease_identifiers_as_numbers` → `tests/updater.rs::is_newer_compares_numeric_prerelease_identifiers_as_numbers`
- `test_is_newer_ranks_alphanumeric_prerelease_identifiers_above_numeric_ones` → `tests/updater.rs::is_newer_ranks_alphanumeric_prerelease_identifiers_above_numeric_ones`
- `test_build_metadata_does_not_affect_precedence` → `tests/updater.rs::build_metadata_does_not_affect_precedence`
- `test_is_newer_is_false_when_either_side_is_unparsable` → `tests/updater.rs::is_newer_is_false_when_either_side_is_unparsable`
- `test_fetch_latest_release_reads_tag_and_url` → `tests/updater.rs::fetch_latest_release_reads_tag_and_url`
- `test_fetch_latest_release_returns_none_when_no_release_exists` → `tests/updater.rs::fetch_latest_release_returns_none_when_no_release_exists`
- `test_fetch_latest_release_raises_on_other_status_codes` → `tests/updater.rs::fetch_latest_release_errors_on_other_status_codes`
- `test_fetch_latest_release_raises_when_connection_fails` → `tests/updater.rs::fetch_latest_release_errors_when_connection_fails`
- `test_fetch_latest_release_raises_on_unusable_payload` → `tests/updater.rs::fetch_latest_release_errors_on_unusable_payload`
- `test_fetch_latest_release_falls_back_to_releases_page_without_html_url` → `tests/updater.rs::fetch_latest_release_falls_back_to_releases_page_without_html_url`
- `test_check_for_update_returns_release_only_when_newer` → `tests/updater.rs::check_for_update_returns_release_only_when_newer`
- `test_check_for_update_returns_none_when_no_release_exists` → `tests/updater.rs::check_for_update_returns_none_when_no_release_exists`
- `test_announce_update_queues_the_banner_when_newer_exists` → `tests/updater.rs::schedule_update_checks_checks_now_and_again_after_the_interval`
- `test_announce_update_stays_quiet_without_a_newer_release` → `tests/updater.rs::found_is_not_called_without_a_newer_release`
- `test_announce_update_swallows_check_failures` → `tests/updater.rs::scheduled_check_survives_failures_and_runs_again`
- `test_schedule_update_checks_checks_now_and_again_after_the_interval` → `tests/updater.rs::schedule_update_checks_checks_now_and_again_after_the_interval`

## tests/test_keep_foreground.py

- `test_the_scan_finds_the_modules_that_raise_windows` → 不移植：Python 測試框架機制（conftest 掃描模組並 monkeypatch `force_foreground`／游標位置）；Rust 以注入式的 `TypingEnv`／`Foreground` 取代，測試本來就不呼叫真實的 SetForegroundWindow
- `test_every_module_binding_force_foreground_is_stubbed_during_tests` → 不移植：Python 測試框架機制（conftest 掃描模組並 monkeypatch `force_foreground`／游標位置）；Rust 以注入式的 `TypingEnv`／`Foreground` 取代，測試本來就不呼叫真實的 SetForegroundWindow
- `test_tk_focus_force_cannot_take_the_foreground_during_tests` → 不移植：Python 測試框架機制（conftest 掃描模組並 monkeypatch `force_foreground`／游標位置）；Rust 以注入式的 `TypingEnv`／`Foreground` 取代，測試本來就不呼叫真實的 SetForegroundWindow
- `test_every_module_binding_cursor_position_gets_the_fixed_cursor` → 不移植：Python 測試框架機制（conftest 掃描模組並 monkeypatch `force_foreground`／游標位置）；Rust 以注入式的 `TypingEnv`／`Foreground` 取代，測試本來就不呼叫真實的 SetForegroundWindow

## tests/test_main.py

- `test_splash_closes_before_focusing_an_existing_instance` → `tests/app_main.rs::second_instance_focuses_and_exits`
- `test_startup_path_does_not_import_anthropic` → 不移植：Python 套件專屬（`anthropic` 套件的匯入時間）；Rust 沒有套件匯入
- `test_run_py_updates_splash_before_importing_main` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_main_py_updates_splash_with_the_starting_phase_constant` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_each_slot_gets_its_own_translator_and_the_cache_follows_incoming` → `tests/app_main.rs::build_translation_prepares_examples_for_incoming_and_region_only`, `tests/app_main.rs::both_pools_translate_through_the_incoming_service`
- `test_the_system_pool_translates_through_the_incoming_service` → `tests/app_main.rs::the_system_pool_translates_through_the_incoming_service`
- `test_changing_only_the_region_slot_leaves_incoming_and_the_cache_alone` → `tests/app_main.rs::changing_only_the_region_slot_leaves_incoming_and_the_cache_alone`
- `test_changing_the_incoming_slot_invalidates_the_cache` → `tests/app_main.rs::changing_the_incoming_slot_invalidates_the_cache`
- `test_build_translation_ensures_examples_for_incoming_and_region` → `tests/app_main.rs::build_translation_prepares_examples_for_incoming_and_region_only`
- `test_reconfigure_ensures_examples_only_for_rebuilt_slots` → `tests/app_main.rs::reconfigure_ensures_examples_only_for_rebuilt_slots`
- `test_reconfigure_without_changes_keeps_examples_and_does_not_ensure` → `tests/app_main.rs::reconfigure_without_changes_keeps_examples_and_the_cache`
- `test_restart_keeps_the_persisted_cache_when_examples_are_already_stored` → `tests/app_main.rs::restart_keeps_the_persisted_cache_when_examples_are_already_stored`
- `test_a_settings_change_applies_the_new_fingerprints_examples_and_never_the_old` → `tests/app_main.rs::a_stale_generation_is_never_applied_and_lands_only_under_the_old_fingerprint`, `tests/app_main.rs::late_examples_after_a_save_bind_the_cache_to_the_new_fingerprint`
- `test_applying_incoming_examples_rebinds_the_system_message_cache` → `tests/app_main.rs::applying_incoming_examples_rebinds_the_system_message_cache`
- `test_build_app_wires_each_consumer_to_its_own_slot` → `tests/app_main.rs::build_app_wires_each_consumer_to_its_own_slot`
- `test_saving_unrelated_settings_rebuilds_no_translator` → `tests/app_main.rs::saving_unrelated_settings_rebuilds_no_translator`
- `test_changing_only_the_region_slot_rebuilds_only_that_translator` → `tests/app_main.rs::changing_only_the_region_slot_leaves_incoming_and_the_cache_alone`
- `test_editing_the_service_a_slot_uses_rebuilds_it` → `tests/app_main.rs::editing_the_service_a_slot_uses_rebuilds_it`
- `test_changing_the_target_language_rebuilds_every_translator` → `tests/app_main.rs::changing_the_target_language_rebuilds_every_translator`
- `test_switching_the_incoming_slot_between_same_named_models_invalidates_the_cache` → `tests/app_main.rs::switching_the_incoming_slot_between_same_named_models_invalidates_the_cache`
- `test_build_app_spawns_readers_through_the_supervisor` → `tests/app_main.rs::build_app_spawns_readers_through_the_supervisor`

## tests/test_no_hardcoded_ui_text.py

- `test_no_hardcoded_cjk_string_literals` → `tests/no_hardcoded_ui_text.rs::no_hardcoded_cjk_string_literals`

## tests/test_parallel_order.py

- `test_display_order_follows_read_order_not_completion_order` → `tests/pool.rs::display_order_independent_of_completion`, `src/ui/overlay/model.rs::update_fills_in_place_keeps_order`

## tests/test_ui_pump.py

- `test_raising_callback_does_not_propagate` → 不移植：Python 的 UI 佇列放的是回呼（closure），一個回呼拋錯就要擋住不影響其餘；Rust 的 UI 佇列放的是 `UiEvent` 資料，由 `App` 每幀排空後依型別處理，沒有「回呼拋錯」這個情境
- `test_later_callbacks_still_run_after_one_raises` → 不移植：Python 的 UI 佇列放的是回呼（closure），一個回呼拋錯就要擋住不影響其餘；Rust 的 UI 佇列放的是 `UiEvent` 資料，由 `App` 每幀排空後依型別處理，沒有「回呼拋錯」這個情境
- `test_empty_queue_is_a_noop` → `src/ui/app.rs::empty_queue_is_a_noop`

## tests/test_version.py

- `test_version_is_semver` → `tests/version.rs::version_is_semver`
- `test_release_workflow_tags_are_accepted_here` → `tests/version.rs::release_workflow_tags_are_accepted_here`
- `test_pyproject_version_matches_package` → 不移植：Python 套件專屬（`pyproject.toml` 與 `src/__init__.py` 兩處版本號要釘在一起）；Rust 只有 `Cargo.toml` 一個來源，執行期以 `env!("CARGO_PKG_VERSION")` 於編譯時讀出

## tests/test_window_parking.py

- `test_windows_are_parked_offscreen` → 不移植：Tk 專屬（Python 的 Tk 視窗測試框架）；Rust 版沒有 Tk（conftest 把 Tk 視窗停到螢幕外）
- `test_parking_keeps_the_requested_size` → 不移植：Tk 專屬（Python 的 Tk 視窗測試框架）；Rust 版沒有 Tk（conftest 把 Tk 視窗停到螢幕外）
- `test_marked_tests_keep_their_real_position` → 不移植：Tk 專屬（Python 的 Tk 視窗測試框架）；Rust 版沒有 Tk（conftest 把 Tk 視窗停到螢幕外）
- `test_marked_windows_are_invisible` → 不移植：Tk 專屬（Python 的 Tk 視窗測試框架）；Rust 版沒有 Tk（conftest 把 Tk 視窗停到螢幕外）

## tests/tools/test_golden_export.py

- `test_check_detects_drift` → 不移植：golden fixture 匯出腳本（`tools/golden/export.py`）是 Python 工具，第 5 段 Task 4 隨 Python 一併刪除；fixture 留在 `tests/golden/`，Rust 端由各 `*_golden` 測試經 `common::golden` 讀取，檔案遺失或格式錯誤會直接 panic

## tests/tools/test_ocr_compare.py

- `test_report_marks_differences_and_counts` → 不移植：Python 新舊 OCR 對照報告工具（`tools/ocr_compare.py`）的測試；Python 版刪除後沒有可比對的另一側，Rust 的 `examples/ocr_compare.rs` 只負責辨識並輸出 JSON、沒有報告邏輯
- `test_report_covers_union_and_marks_missing` → 不移植：Python 新舊 OCR 對照報告工具（`tools/ocr_compare.py`）的測試；Python 版刪除後沒有可比對的另一側，Rust 的 `examples/ocr_compare.rs` 只負責辨識並輸出 JSON、沒有報告邏輯
- `test_report_exits_with_hint_when_json_missing` → 不移植：Python 新舊 OCR 對照報告工具（`tools/ocr_compare.py`）的測試；Python 版刪除後沒有可比對的另一側，Rust 的 `examples/ocr_compare.rs` 只負責辨識並輸出 JSON、沒有報告邏輯

## tests/tools/test_splash_image.py

- `test_generates_a_png_of_the_expected_size` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_creates_missing_parent_directories` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_version_is_drawn_into_the_image` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）

## tests/tools/test_splash_progress.py

- `test_size_table_maps_lowercased_basename_to_byte_size_list` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_size_table_skips_entries_whose_source_is_unreadable` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_size_table_skips_basenames_with_tcl_brace_characters` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_size_table_keeps_both_sizes_on_basename_collision` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_size_table_combines_binaries_and_datas` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_size_table_tcl_braces_each_basename_and_its_size_list` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_returns_the_total_bytes_across_binaries_and_datas` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_injects_the_size_table_and_bar_elements` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_raises_if_nothing_has_a_readable_size` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_raises_if_canvas_anchor_is_missing` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_raises_if_text_update_anchor_is_missing` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_raises_if_canvas_anchor_appears_twice` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_raises_if_text_update_anchor_appears_twice` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_install_progress_bar_does_not_double_inject_on_repeated_calls` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_progress_tracking_pops_one_size_per_report_in_a_real_tcl_interpreter` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_extraction_target_is_capped_at_the_reserved_ceiling_not_full_width` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_phase_strings_move_the_target_to_their_reserved_percentage` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_progress_step_advances_on_a_single_direct_call_with_no_after_ever_firing` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_progress_step_after_chain_converges_when_the_event_loop_actually_runs` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）
- `test_progress_step_cancels_the_previous_after_before_scheduling_a_new_one` → 不移植：PyInstaller／啟動畫面專屬；Rust 版是單一 exe、不需要啟動畫面（規格「單一 exe 與資源」節）

## tests/reader/test_chat_hook.py

stub 的暫存器還原、SSO／堆積字串複製、截斷與環繞寫入原本靠 unicorn 模擬 x86 執行驗證；Rust 版不引入 x86 模擬器，改以 golden 釘死 stub 位元組與 Python 版完全相同（Python 的模擬已驗證過該位元組串的行為），stub 執行行為本身要靠實機驗證。

- `test_stub_copies_sso_and_heap_strings_and_restores_registers` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`（僅釘機器碼與 Python 逐位元組相同；執行行為由 Python 的模擬器測試證明，Rust 端靠實機驗證）
- `test_stub_truncates_long_lines_and_keeps_original_length` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`（僅釘機器碼與 Python 逐位元組相同；執行行為由 Python 的模擬器測試證明，Rust 端靠實機驗證）
- `test_stub_wraps_past_the_slot_count` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`（僅釘機器碼與 Python 逐位元組相同；執行行為由 Python 的模擬器測試證明，Rust 端靠實機驗證）
- `test_read_ring_returns_new_records_in_order` → `tests/reader_golden.rs::read_ring_returns_new_records_in_order`
- `test_read_ring_stops_at_a_slot_being_written` → `tests/reader_golden.rs::read_ring_stops_at_a_slot_being_written_or_still_on_the_previous_lap`
- `test_read_ring_waits_for_a_slot_still_holding_the_previous_lap` → `tests/reader_golden.rs::read_ring_stops_at_a_slot_being_written_or_still_on_the_previous_lap`
- `test_read_ring_skips_the_content_read_for_an_empty_line` → `tests/reader_golden.rs::read_ring_skips_the_content_read_for_an_empty_line`
- `test_read_ring_counts_an_overwritten_slot_as_lost` → `tests/reader_golden.rs::read_ring_counts_an_overwritten_slot_as_lost`
- `test_overflow_reports_lost_and_resumes_at_oldest_valid_slot` → `tests/reader_golden.rs::read_ring_overflow_reports_lost_and_resumes_at_oldest_valid_slot`
- `test_read_ring_rereads_seq_to_detect_a_torn_read` → `tests/reader_golden.rs::read_ring_rereads_seq_to_detect_a_torn_read`
- `test_group_broadcasts_collapses_tabs_0_to_3_plus_channels` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_group_broadcasts_collapses_tabs_0_to_3_without_channels` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_group_broadcasts_leaves_team_up_tab_out_of_a_broadcast` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_group_broadcasts_keeps_ordinary_lines_apart` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_pending_broadcast_counts_a_trailing_broadcast_prefix` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_pending_broadcast_ignores_lone_lines_and_complete_broadcasts` → `tests/reader_golden.rs::group_broadcasts_matches_python`
- `test_unhook_restores_the_jump_but_keeps_the_ring` → `tests/hooks_golden.rs::ring_is_never_freed`, `tests/mem_reader.rs::close_unhooks_and_clears_hook_state`
- `test_bytecode_generator_builds_the_stub_for_the_allocated_ring` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`

## tests/reader/test_chat_tabs.py

- `test_read_wstring_inline_short_string` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_five_chars_inline` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_heap_string` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_empty_and_absurd_sizes` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_size_limit_boundary` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_sso_capacity_threshold` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_read_wstring_size_beyond_capacity_is_rejected` → `src/reader/chat_tabs.rs::read_label_inline_heap_and_rejected_sizes`
- `test_visible_label_hides_unresolved_tokens` → `tests/reader_golden.rs::visible_label_matches_python`
- `test_visible_label_keeps_unread_suffix` → `tests/reader_golden.rs::visible_label_matches_python`
- `test_tab_buttons_cover_the_verified_mapping` → `src/reader/chat_tabs.rs::tab_buttons_cover_the_verified_mapping`

## tests/reader/test_hook_state.py

- `test_save_load_roundtrip` → `src/reader/hook_state.rs::save_load_roundtrip`
- `test_load_missing_returns_empty` → `src/reader/hook_state.rs::load_missing_returns_empty`
- `test_clear_removes_file` → `src/reader/hook_state.rs::clear_removes_file_and_missing_is_noop`
- `test_clear_missing_is_noop` → `src/reader/hook_state.rs::clear_removes_file_and_missing_is_noop`
- `test_corrupt_file_returns_empty` → `tests/hooks_golden.rs::corrupt_state_file_is_logged_and_ignored`
- `test_empty_ops_roundtrip` → `src/reader/hook_state.rs::empty_ops_roundtrip`
- `test_sweep_removes_dead_pid_files_only` → `tests/hooks_golden.rs::sweep_removes_dead_pids_only`
- `test_sweep_ignores_non_state_files` → `tests/hooks_golden.rs::sweep_removes_dead_pids_only`

## tests/reader/test_input_anchor.py

- `test_client_rect_scales_summed_offsets_by_client_over_root` → `tests/reader_golden.rs::client_rect_matches_python`
- `test_client_rect_rounds_instead_of_truncating` → `tests/reader_golden.rs::client_rect_matches_python`
- `test_client_rect_is_identity_when_root_matches_client` → `tests/reader_golden.rs::client_rect_matches_python`
- `test_client_rect_rejects_degenerate_root` → `tests/reader_golden.rs::client_rect_matches_python`, `tests/mem_reader.rs::input_box_screen_rect_none_for_a_degenerate_root`
- `test_input_box_screen_rect_walks_parents_and_offsets_by_window_origin` → `tests/mem_reader.rs::input_box_screen_rect_walks_parents_and_offsets_by_window_origin`
- `test_input_box_screen_rect_none_when_node_read_fails` → `tests/mem_reader.rs::input_box_screen_rect_none_when_node_read_fails`
- `test_input_box_screen_rect_none_when_not_connected` → `tests/mem_reader.rs::tab_queries_are_empty_when_not_connected`

## tests/reader/test_loop.py

- `test_system_lines_skip_the_chat_context` → `tests/reader_loop.rs::system_lines_skip_the_chat_context`
- `test_cached_system_line_is_added_already_translated` → `tests/reader_loop.rs::system_cache_hit_shown_without_pool`
- `test_uncached_system_line_goes_to_the_system_pool` → `tests/reader_loop.rs::uncached_system_line_goes_to_the_system_pool`
- `test_banner_shows_a_system_pool_error_when_the_player_pool_is_clean` → `tests/reader_loop.rs::banner_shows_a_system_pool_error_when_the_player_pool_is_clean`
- `test_system_lines_are_ignored_when_the_setting_is_off` → `tests/reader_loop.rs::system_lines_are_ignored_when_the_setting_is_off`
- `test_reader_emit_system_is_resynced_every_loop_iteration` → `tests/reader_loop.rs::reader_emit_system_is_resynced_every_loop_iteration`
- `test_new_lines_are_placeheld_and_submitted_in_order` → `tests/reader_loop.rs::new_lines_are_placeheld_and_submitted_in_order`
- `test_game_color_flows_to_overlay_text_to_context_and_pool` → `tests/reader_loop.rs::game_color_flows_to_overlay_text_to_context_and_pool`
- `test_context_advances_by_read_order_not_by_completion` → `tests/reader_loop.rs::player_line_context_snapshot_before_push`
- `test_banner_follows_pool_error_state` → `tests/reader_loop.rs::banner_follows_pool_error_state`
- `test_banner_prefers_game_issue_over_translation_error` → `tests/status.rs::banner_prefers_game_issue_over_translation_error`
- `test_banner_shows_the_api_message_when_there_is_one` → `tests/status.rs::banner_shows_the_api_message_when_there_is_one`
- `test_loop_passes_the_pool_error_detail_to_the_banner` → `tests/reader_loop.rs::loop_passes_the_pool_error_detail_to_the_banner`
- `test_status_shows_translating_while_pool_busy` → `tests/reader_loop.rs::status_shows_translating_while_pool_busy`
- `test_status_transitions` → `tests/reader_loop.rs::status_transitions`
- `test_status_locating_when_not_anchored` → `tests/reader_loop.rs::status_locating_when_not_anchored`
- `test_game_input_edge_triggers_open_and_close` → `tests/reader_loop.rs::input_open_and_close_are_posted_with_the_anchor`, `tests/reader_loop.rs::input_events_are_edge_triggered`
- `test_game_input_edges_are_reported_even_when_auto_show_is_off` → `tests/reader_loop.rs::input_open_and_close_are_posted_with_the_anchor`
- `test_game_not_running_shows_banner_once` → `tests/reader_loop.rs::game_missing_retries_every_5s_and_logs_once_per_issue`
- `test_access_denied_shows_its_own_banner_and_status` → `tests/reader_loop.rs::game_missing_retries_every_5s_and_logs_once_per_issue`
- `test_version_mismatch_shows_its_own_banner_and_status` → `tests/reader_loop.rs::version_mismatch_shows_its_own_banner_and_status`
- `test_game_input_detected_during_the_wait_between_polls` → `tests/reader_loop.rs::input_box_is_sampled_during_the_wait_between_polls`
- `test_loop_ends_and_closes_the_reader_when_its_window_is_gone` → `tests/reader_loop.rs::window_gone_stops_reader`
- `test_messages_carry_the_slot` → `tests/reader_loop.rs::new_lines_are_placeheld_and_submitted_in_order`
- `test_input_callbacks_carry_the_window_handle` → `tests/reader_loop.rs::input_open_and_close_are_posted_with_the_anchor`
- `test_message_ids_are_unique_across_threads` → `tests/reader_loop.rs::message_ids_are_unique_across_threads`
- `test_events_carry_their_tabs_to_the_overlay` → `tests/reader_loop.rs::events_carry_their_tabs_to_the_overlay`
- `test_player_lines_use_their_own_tab_context` → `tests/reader_loop.rs::player_lines_use_their_own_tab_context`
- `test_tab_state_is_sent_only_when_it_changes` → `tests/reader_loop.rs::tab_state_is_sent_only_when_it_changes`
- `test_tab_state_is_cleared_when_the_game_goes_away` → `tests/reader_loop.rs::tab_state_is_cleared_when_the_game_goes_away`
- `test_foreground_client_reports_its_slot_every_poll` → `tests/reader_loop.rs::foreground_slot_posted_on_each_full_sample`
- `test_tab_state_and_foreground_are_sampled_slower_than_chat` → `tests/reader_loop.rs::tab_state_and_foreground_are_sampled_slower_than_chat`
- `test_a_tab_switch_is_reported_without_waiting_for_the_slow_sample` → `tests/reader_loop.rs::a_tab_switch_is_reported_without_waiting_for_the_slow_sample`

## tests/reader/test_markup.py

- `test_clean_strips_markup_and_collapses_space` → `src/reader/markup.rs::clean_strips_markup_and_collapses_space`
- `test_clean_unescapes_player_typed_entities` → `src/reader/markup.rs::clean_keeps_emoticons_and_unescapes_entities`
- `test_clean_keeps_emoticon_as_name` → `src/reader/markup.rs::clean_keeps_emoticons_and_unescapes_entities`
- `test_clean_strips_emoticon_prefix_and_digits` → `src/reader/markup.rs::clean_strips_emoticon_prefix_and_digits`
- `test_lines_extracts_player_say_with_sender` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_cjk_sender_and_spaces` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_flags_system_messages` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_skips_debug_rows` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_own_message` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_own_and_others_together` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_preserves_order_and_repeats` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_emoticon_line` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_astral_emoji_text` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_empty_for_empty_input` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_house_channel_message_with_balloon_icon` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_keeps_house_guest_variant` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_skips_debug_rows_without_icon` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_system_lines_are_emitted_with_the_system_flag` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_player_lines_are_not_flagged_as_system` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_system_lines_do_not_need_a_sender_prefix` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_party_notices_are_system_lines` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_party_chat_stays_a_player_line` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_system_lines_that_clean_to_nothing_are_dropped` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_debug_lines_are_still_dropped` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_server_broadcast_without_any_icon_is_flagged_as_system` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_server_broadcast_does_not_need_a_sender_prefix` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_server_broadcast_that_cleans_to_nothing_is_dropped` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_iconless_lines_without_a_leading_color_tag_are_still_dropped` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_with_an_unknown_art_icon_are_not_taken_as_broadcasts` → `tests/reader_golden.rs::parse_line_matches_python`, `src/reader/markup.rs::unknown_icons_are_warned_once_per_icon`
- `test_system_and_player_lines_keep_their_in_game_order` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_carry_game_color` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_color_normalized_lowercase_hex` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_color_takes_last_six_of_eight_digit_hex` → `tests/reader_golden.rs::parse_line_matches_python`
- `test_lines_color_missing_is_none` → `tests/reader_golden.rs::parse_line_matches_python`

## tests/reader/test_message_log.py

- `test_record_writes_seq_tab_verdict_and_raw_markup` → `src/reader/message_log.rs::record_writes_seq_tab_verdict_and_raw_markup`
- `test_note_writes_the_text_as_is` → `src/reader/message_log.rs::note_writes_the_text_as_is`
- `test_debug_lines_are_not_recorded` → `src/reader/message_log.rs::debug_lines_are_not_recorded`
- `test_game_debug_tags_are_recognised` → `src/reader/message_log.rs::game_debug_tags_are_recognised`
- `test_chat_lines_are_not_debug_lines` → `src/reader/message_log.rs::chat_lines_are_not_debug_lines`
- `test_slot_prefix_marks_every_line` → `src/reader/message_log.rs::slot_prefix_marks_every_line`

## tests/reader/test_reader_connect.py

- `test_open_process_denied_raises_access_denied` → `tests/mem_reader.rs::open_failure_is_access_denied`
- `test_access_denied_is_a_game_not_running` → `tests/reader_loop.rs::game_missing_retries_every_5s_and_logs_once_per_issue`
- `test_other_connect_failure_stays_game_not_running` → `tests/mem_reader.rs::open_failure_is_access_denied`, `tests/mem_reader.rs::other_attach_failure_stays_game_not_running`
- `test_failed_connect_closes_event_loop` → 不移植：Python 專屬機制；Python 版每次連線建一個 asyncio event loop，失敗要手動收掉，Rust 版是同步呼叫、沒有 event loop 可洩漏
- `test_pattern_failures_are_classified_as_version_mismatch` → `tests/mem_reader.rs::pattern_failure_is_version_mismatch_and_leaves_no_partial_hook`
- `test_hook_ready_timeout_is_classified_as_version_mismatch` → `tests/mem_reader.rs::readiness_timeout_is_version_mismatch`
- `test_unrelated_connect_failures_are_not_version_mismatch` → `tests/mem_reader.rs::other_attach_failure_stays_game_not_running`, `tests/mem_reader.rs::open_failure_is_access_denied`
- `test_version_mismatch_is_a_game_not_running` → `tests/reader_loop.rs::version_mismatch_shows_its_own_banner_and_status`, `tests/reader_loop.rs::game_missing_retries_every_5s_and_logs_once_per_issue`
- `test_wait_root_window_ready_returns_once_the_address_is_set` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_wait_root_window_ready_ignores_transient_read_errors` → `tests/mem_reader.rs::connect_waits_through_transient_read_errors`
- `test_wait_root_window_ready_times_out_instead_of_hanging_forever` → `tests/mem_reader.rs::readiness_timeout_is_version_mismatch`
- `test_pattern_failure_while_attaching_raises_version_mismatch` → `tests/mem_reader.rs::pattern_failure_is_version_mismatch_and_leaves_no_partial_hook`
- `test_chat_hook_pattern_failure_raises_version_mismatch` → `tests/mem_reader.rs::pattern_failure_is_version_mismatch_and_leaves_no_partial_hook`
- `test_hook_never_firing_raises_version_mismatch` → `tests/mem_reader.rs::readiness_timeout_is_version_mismatch`
- `test_other_attach_failure_stays_game_not_running` → `tests/mem_reader.rs::other_attach_failure_stays_game_not_running`
- `test_hook_state_is_saved_before_waiting_for_the_hook_to_fire` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_attach_saves_state_around_each_hook_and_waits_before_addline` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_addline_is_not_installed_when_the_root_hook_never_fires` → `tests/mem_reader.rs::readiness_timeout_is_version_mismatch`
- `test_addline_pattern_failure_names_the_addline_hook_in_the_log` → `tests/mem_reader.rs::pattern_failure_is_version_mismatch_and_leaves_no_partial_hook`
- `test_attach_does_not_use_wizwalkers_unbounded_wait` → 不移植：Python 套件專屬；測的是不使用 wizwalker 內建的無限等待（`wait_for_ready`），Rust 版沒有 wizwalker，自己的等待逾時由 `tests/mem_reader.rs::readiness_timeout_is_version_mismatch` 覆蓋
- `test_connect_binds_the_given_window_handle` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_connect_logs_the_path_from_the_clients_own_pid` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_teardown_closes_the_client` → `tests/mem_reader.rs::close_unhooks_and_clears_hook_state`, `tests/mem_reader.rs::dropping_the_reader_unhooks_like_close`
- `test_install_path_of_derives_the_install_root_from_the_pid` → `tests/mem_reader.rs::install_path_is_the_parent_of_bin`
- `test_install_path_of_rejects_a_non_game_path` → `tests/mem_reader.rs::install_path_is_the_parent_of_bin`
- `test_install_path_of_handles_an_unopenable_process` → `tests/mem_reader.rs::install_path_is_the_parent_of_bin`
- `test_is_game_process_path_matches_game_exe` → `src/reader/process.rs::game_process_path_matches_file_name_case_insensitively`
- `test_is_game_process_path_rejects_other_or_missing` → `src/reader/process.rs::game_process_path_matches_file_name_case_insensitively`
- `test_find_game_window_returns_the_first_matching_visible_window` → `src/reader/process.rs::find_game_window_returns_the_first_matching_visible_window`
- `test_find_game_window_returns_none_when_the_game_is_not_running` → `src/reader/process.rs::find_game_window_returns_none_when_the_game_is_not_running`
- `test_find_game_window_skips_invisible_windows` → `src/reader/process.rs::find_game_window_skips_invisible_windows`
- `test_find_game_window_ignores_a_single_window_query_failure` → `src/reader/process.rs::find_game_window_ignores_a_single_window_query_failure`
- `test_repair_reports_write_failures_instead_of_claiming_success` → `tests/hooks_golden.rs::repair_reports_write_failures_instead_of_claiming_success`
- `test_repair_skips_and_logs_a_stale_state_file` → `tests/hooks_golden.rs::repair_skips_and_logs_a_stale_state_file`
- `test_hook_state_save_failure_is_logged` → `tests/mem_reader.rs::hook_state_save_failure_is_logged`
- `test_hook_state_saves_hook_jumps_before_autobot` → `tests/mem_reader.rs::connect_order_matches_python`
- `test_find_game_window_logs_when_the_window_scan_itself_fails` → `tests/mem_reader.rs::find_game_window_logs_when_the_window_scan_itself_fails`
- `test_window_exists_distinguishes_a_live_window_from_a_dead_handle` → `tests/mem_reader.rs::install_path_is_the_parent_of_bin`, `src/reader/process.rs::process_queries_handle_missing_targets`

## tests/reader/test_reader_events.py

- `test_records_become_events_in_order` → `tests/mem_reader.rs::records_become_events_in_order`
- `test_identical_consecutive_lines_both_emit` → `tests/mem_reader.rs::identical_consecutive_lines_both_emit`
- `test_system_lines_are_skipped_when_emit_system_off` → `tests/mem_reader.rs::system_lines_suppressed_but_seq_advances`
- `test_broadcast_becomes_one_event_for_all_tabs` → `tests/mem_reader.rs::broadcast_becomes_one_event_for_all_tabs`
- `test_broadcast_split_across_polls_is_held_and_collapsed` → `tests/mem_reader.rs::partial_broadcast_held_one_round`
- `test_a_held_tail_is_released_when_nothing_follows` → `tests/mem_reader.rs::a_held_tail_is_released_when_nothing_follows`
- `test_a_lone_main_tab_line_is_not_held` → `tests/mem_reader.rs::a_lone_main_tab_line_is_not_held`
- `test_unparseable_lines_are_dropped_and_logged` → `tests/mem_reader.rs::unparseable_lines_are_dropped_and_logged`
- `test_unknown_chatbox_triggers_one_refind_before_dropping` → `tests/mem_reader.rs::unknown_this_relooks_chatbox_once_per_round`, `tests/mem_reader.rs::only_wizardchatbox_this_is_accepted`
- `test_missing_chatbox_does_not_mark_the_record_foreign` → `tests/mem_reader.rs::missing_chatbox_does_not_mark_the_record_foreign`
- `test_missing_chatbox_is_refound_once_per_poll` → `tests/mem_reader.rs::missing_chatbox_is_refound_once_per_poll`
- `test_chatbox_moving_clears_foreign_and_button_caches` → `tests/mem_reader.rs::chatbox_moving_clears_foreign_and_button_caches`
- `test_foreign_address_is_rechecked_after_the_window` → `tests/mem_reader.rs::foreign_address_is_rechecked_after_the_window`
- `test_refinding_the_same_chatbox_is_a_no_op` → `tests/mem_reader.rs::refinding_the_same_chatbox_is_a_no_op`
- `test_lost_records_are_logged_with_the_count` → `tests/mem_reader.rs::lost_records_are_logged_with_the_count`
- `test_truncated_lines_are_logged_with_the_original_length` → `tests/mem_reader.rs::truncated_lines_are_logged_with_the_original_length`
- `test_non_bmp_text_is_not_reported_as_truncated` → `tests/mem_reader.rs::non_bmp_text_is_not_reported_as_truncated`
- `test_teardown_resets_the_ring_cursor` → `tests/mem_reader.rs::teardown_resets_the_ring_cursor`
- `test_tab_state_reads_current_tab_and_visible_labels` → `tests/mem_reader.rs::tab_state_reads_current_tab_and_visible_labels`
- `test_tab_hidden_after_being_shown_is_dropped` → `tests/mem_reader.rs::tab_hidden_after_being_shown_is_dropped`
- `test_tab_state_is_none_without_a_chatbox` → `tests/mem_reader.rs::tab_state_is_none_without_a_chatbox`
- `test_repeated_chatbox_lookup_failures_log_once` → `tests/mem_reader.rs::repeated_chatbox_lookup_failures_log_once`
- `test_tab_buttons_are_refound_after_the_chatbox_moves` → `tests/mem_reader.rs::tab_buttons_are_refound_after_the_chatbox_moves`
- `test_no_visible_tab_buttons_are_not_rescanned_every_poll` → `tests/mem_reader.rs::no_visible_tab_buttons_are_not_rescanned_every_poll`
- `test_out_of_range_current_tab_is_a_read_failure` → `tests/mem_reader.rs::out_of_range_current_tab_is_a_read_failure`
- `test_tab_state_is_none_when_not_connected` → `tests/mem_reader.rs::tab_queries_are_empty_when_not_connected`
- `test_current_tab_reads_only_the_current_tab` → `tests/mem_reader.rs::current_tab_reads_only_the_known_chatbox`
- `test_current_tab_is_none_without_a_known_chatbox` → `tests/mem_reader.rs::current_tab_reads_only_the_known_chatbox`
- `test_current_tab_out_of_range_is_none` → `tests/mem_reader.rs::current_tab_reads_only_the_known_chatbox`
- `test_current_tab_is_none_when_not_connected` → `tests/mem_reader.rs::tab_queries_are_empty_when_not_connected`

## tests/reader/test_status.py

- `test_state_priority_translating_beats_everything` → `tests/status.rs::state_priority_translating_beats_everything`
- `test_state_priority_listening_beats_locating_and_failures` → `tests/status.rs::state_priority_listening_beats_locating_and_failures`
- `test_permanent_failure_shows_when_nothing_is_attached` → `tests/status.rs::permanent_failure_shows_when_nothing_is_attached`
- `test_no_slots_means_waiting_for_the_game` → `tests/status.rs::no_slots_means_waiting_for_the_game`
- `test_permanent_issue_is_reported_even_if_another_slot_works` → `tests/status.rs::permanent_issue_wins_and_attached_suppresses_missing`
- `test_transient_failure_on_one_slot_is_hidden_while_another_listens` → `tests/status.rs::permanent_issue_wins_and_attached_suppresses_missing`
- `test_all_slots_failing_reports_game_missing` → `tests/status.rs::all_slots_failing_reports_game_missing`
- `test_no_slots_reports_game_missing` → `tests/status.rs::no_slots_reports_game_missing`
- `test_all_slots_still_locating_shows_no_banner` → `tests/status.rs::all_slots_still_locating_shows_no_banner`
- `test_board_pushes_only_on_change` → `tests/status.rs::board_pushes_only_on_change`
- `test_board_drop_recomputes` → `tests/status.rs::board_drop_recomputes`
- `test_board_banner_follows_game_issue_and_pool_errors` → `tests/status.rs::board_banner_follows_game_issue_and_pool_errors`
- `test_board_refresh_with_no_slots_reports_waiting_game` → `tests/status.rs::board_refresh_with_no_slots_reports_waiting_game`

## tests/reader/test_supervisor.py

- `test_allocate_slot_takes_the_smallest_free_number` → `tests/supervisor.rs::allocate_slot_takes_the_smallest_free_number`
- `test_each_new_window_gets_a_thread_and_the_next_slot` → `tests/supervisor.rs::each_new_window_gets_a_thread_and_the_next_slot`
- `test_freed_slot_is_reused_by_the_next_window` → `tests/supervisor.rs::smallest_free_slot_reused`
- `test_board_is_refreshed_every_scan` → `tests/supervisor.rs::board_is_refreshed_every_scan`
- `test_stop_joins_every_reader_thread` → `tests/supervisor.rs::stop_joins_every_reader_thread`
- `test_enumeration_failure_does_not_kill_the_loop` → `tests/supervisor.rs::scan_failure_does_not_kill_the_loop`
- `test_shutdown_join_budget_is_shared_not_per_thread` → `tests/supervisor.rs::join_budget_shared_8s`

## tests/translation/test_cache.py

- `test_normalize_replaces_numbers_with_placeholders` → `tests/cache_golden.rs::normalize_matches_python`
- `test_normalize_numbers_the_placeholders_in_order` → `tests/cache_golden.rs::normalize_matches_python`
- `test_normalize_keeps_decimals_and_thousands_separators_whole` → `tests/cache_golden.rs::normalize_matches_python`
- `test_normalize_leaves_text_without_numbers_untouched` → `tests/cache_golden.rs::normalize_matches_python`
- `test_restore_puts_the_numbers_back` → `tests/cache_golden.rs::normalize_matches_python`
- `test_restore_follows_the_translated_word_order` → `tests/cache_golden.rs::restore_follows_placeholder_numbers_not_positions`
- `test_normalize_restore_round_trips` → `tests/cache_golden.rs::normalize_matches_python`
- `test_placeholders_match_accepts_a_faithful_translation` → `tests/cache_golden.rs::placeholders_match_checks_counts`
- `test_placeholders_match_rejects_a_dropped_placeholder` → `tests/cache_golden.rs::placeholders_match_checks_counts`
- `test_placeholders_match_rejects_an_invented_placeholder` → `tests/cache_golden.rs::placeholders_match_checks_counts`
- `test_placeholders_match_rejects_a_duplicated_placeholder` → `tests/cache_golden.rs::placeholders_match_checks_counts`
- `test_get_returns_none_when_empty` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`
- `test_put_then_get_round_trips_with_the_number_restored` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`
- `test_a_different_number_hits_the_same_entry` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`
- `test_put_rejects_a_translation_with_broken_placeholders` → `tests/cache_golden.rs::put_rejects_broken_placeholders`
- `test_put_expects_the_translation_of_the_normalized_template` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`, `tests/cache_golden.rs::flush_bytes_match_python`
- `test_put_takes_the_raw_text_not_the_template` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`（含「傳樣板查不到」的斷言）
- `test_evicts_the_least_recently_used_entry` → `tests/cache_golden.rs::lru_caps_at_2000`（Rust 以常數 2000 為上限測試，不像 Python 以 monkeypatch 縮小上限）
- `test_flush_writes_the_file_with_the_fingerprint` → `tests/cache_golden.rs::flush_bytes_match_python`
- `test_load_restores_entries_from_disk` → `tests/cache_golden.rs::loads_python_written_cache`
- `test_load_discards_everything_when_the_fingerprint_differs` → `tests/cache_golden.rs::load_discards_mismatched_corrupt_and_malformed_files`
- `test_load_survives_a_corrupt_file` → `tests/cache_golden.rs::load_discards_mismatched_corrupt_and_malformed_files`
- `test_load_survives_a_structurally_malformed_file` → `tests/cache_golden.rs::load_discards_mismatched_corrupt_and_malformed_files`
- `test_load_truncates_to_the_cap_and_logs_the_stored_count` → `tests/cache_golden.rs::load_truncates_to_the_cap_keeping_the_newest`（本批補上 log 筆數斷言）
- `test_load_survives_a_missing_file` → `tests/cache_golden.rs::load_discards_mismatched_corrupt_and_malformed_files`
- `test_autoflushes_after_enough_new_entries` → `tests/cache_golden.rs::flushes_every_20_puts`
- `test_rebind_flushes_the_old_fingerprint_and_starts_empty` → `tests/cache_golden.rs::fingerprint_change_clears_on_rebind`
- `test_put_accepts_a_translation_produced_under_the_current_fingerprint` → `tests/cache_golden.rs::put_get_round_trips_and_other_numbers_hit_the_same_entry`, `tests/cache_golden.rs::fingerprint_change_clears_on_rebind`
- `test_put_rejects_a_translation_produced_under_a_previous_fingerprint` → `tests/cache_golden.rs::fingerprint_change_clears_on_rebind`, `tests/cache_golden.rs::does_not_store_when_fingerprint_changed_mid_flight`
- `test_rebind_is_a_no_op_when_the_fingerprint_is_unchanged` → `tests/cache_golden.rs::rebind_to_the_same_fingerprint_is_a_no_op`
- `test_translate_and_cache_stores_the_normalized_template` → `tests/cache_golden.rs::translate_and_cache_stores_the_normalized_template`
- `test_translate_and_cache_falls_back_when_the_model_mangles_a_placeholder` → `tests/cache_golden.rs::does_not_store_on_placeholder_mismatch_and_translates_original`
- `test_translate_and_cache_discards_a_translation_that_finished_after_a_rebind` → `tests/cache_golden.rs::does_not_store_when_fingerprint_changed_mid_flight`
- `test_fingerprint_never_contains_the_api_key` → `tests/cache_golden.rs::fingerprint_never_contains_the_api_key`（本批新增；`fingerprint_of` 本來就沒有金鑰參數）
- `test_cache_file_never_contains_the_api_key` → `tests/cache_golden.rs::cache_file_never_contains_the_api_key`（本批新增）
- `test_clear_empties_the_cache_and_reports_the_count` → `tests/cache_golden.rs::clear_removes_entries_and_file_without_writing_back`
- `test_clear_removes_the_file_from_disk` → `tests/cache_golden.rs::clear_removes_entries_and_file_without_writing_back`
- `test_clear_survives_a_missing_file` → `tests/cache_golden.rs::clear_removes_entries_and_file_without_writing_back`
- `test_clear_does_not_write_the_discarded_entries_back` → `tests/cache_golden.rs::clear_removes_entries_and_file_without_writing_back`
- `test_cache_still_usable_after_clear` → `tests/cache_golden.rs::cache_still_usable_after_clear`（本批新增）
- `test_translate_and_cache_does_not_store_a_translation_that_slipped_into_english` → `tests/cache_golden.rs::does_not_store_stray_latin`
- `test_translate_and_cache_stores_a_translation_that_kept_the_source_english` → `tests/cache_golden.rs::stores_a_translation_that_kept_the_source_english`
- `test_translate_and_cache_judges_with_the_target_captured_before_the_request` → `tests/cache_golden.rs::judges_with_the_target_captured_before_the_request`（本批新增）
- `test_fingerprint_covers_the_prompt_revision` → `tests/cache_golden.rs::fingerprint_matches_python`（golden 期望值含 `p6`，即提示詞版次）
- `test_fingerprint_separates_two_endpoints_that_share_a_model_name` → `tests/cache_golden.rs::fingerprint_matches_python`（golden 含帶端點網址的案例）
- `test_fingerprint_includes_the_examples_digest` → `tests/cache_golden.rs::fingerprint_matches_python`（golden 含 `x-` 與 `xabc…` 兩種結尾）

## tests/translation/test_clients.py

- `test_openai_compat_sends_temperature_zero` → `tests/clients_openai.rs::custom_keeps_max_tokens_and_temperature_by_thinking_mode`, `tests/clients_golden.rs::request_body_matches_python`
- `test_openai_compat_connection_error_maps_to_offline` → `tests/clients_openai.rs::connection_failure_is_offline_without_status`
- `test_openai_compat_auth_or_model_error_maps_to_config_error` → `tests/clients_openai.rs::status_4xx_is_config_with_detail`
- `test_openai_compat_retryable_status_maps_to_offline` → `tests/clients_openai.rs::status_429_and_5xx_are_offline`
- `test_claude_provider_returns_text` → `tests/clients_claude.rs::claude_sends_structured_output_and_parses_translation`
- `test_claude_marks_the_system_prompt_as_the_cache_breakpoint` → `tests/clients_claude.rs::claude_system_has_cache_control_at_end`, `tests/clients_claude.rs::claude_sends_structured_output_and_parses_translation`
- `test_claude_logs_prompt_cache_usage` → `tests/clients_claude.rs::claude_logs_cache_read_write_uncached`
- `test_claude_connection_error_maps_to_offline` → `tests/clients_claude.rs::claude_retries_connection_failures_then_offline`
- `test_claude_status_error_mapping` → `tests/clients_claude.rs::claude_retry_conditions_follow_the_sdk`, `tests/clients_claude.rs::claude_retries_429_twice_then_offline`
- `test_openai_compat_sends_max_tokens_by_thinking_mode` → `tests/clients_openai.rs::custom_keeps_max_tokens_and_temperature_by_thinking_mode`, `tests/clients_golden.rs::request_body_matches_python`
- `test_openai_compat_truncated_output_maps_to_bad_output` → `tests/clients_openai.rs::length_finish_is_bad_output_with_sample`
- `test_openai_compat_logs_cached_prompt_tokens` → `tests/clients_openai.rs::logs_prompt_cache_usage`
- `test_openai_compat_missing_finish_reason_is_accepted` → `tests/clients_openai.rs::missing_finish_reason_is_accepted`
- `test_claude_sends_max_tokens` → `tests/clients_claude.rs::claude_sends_structured_output_and_parses_translation`
- `test_claude_auto_effort_sends_no_effort` → `tests/clients_claude.rs::claude_effort_only_when_not_auto`
- `test_claude_low_effort_sends_output_config` → `tests/clients_claude.rs::claude_effort_only_when_not_auto`
- `test_claude_truncated_output_maps_to_bad_output` → `tests/clients_claude.rs::claude_max_tokens_stop_is_bad_output`
- `test_claude_reads_the_first_text_block` → `tests/clients_claude.rs::claude_reads_the_first_text_block`
- `test_claude_unexpected_output_maps_to_bad_output` → `tests/clients_claude.rs::claude_unexpected_output_is_bad_output`
- `test_openai_provider_disables_thinking_with_official_param_only` → `tests/clients_golden.rs::request_body_matches_python`, `tests/clients_openai.rs::official_uses_max_completion_tokens_and_no_temperature`
- `test_openai_provider_thinking_on_sends_no_thinking_params` → `tests/clients_golden.rs::request_body_matches_python`
- `test_openai_provider_uses_max_completion_tokens_and_no_temperature` → `tests/clients_openai.rs::official_uses_max_completion_tokens_and_no_temperature`, `tests/clients_golden.rs::request_body_matches_python`
- `test_custom_endpoint_keeps_max_tokens_and_temperature` → `tests/clients_openai.rs::custom_keeps_max_tokens_and_temperature_by_thinking_mode`, `tests/clients_golden.rs::request_body_matches_python`
- `test_rejected_parameter_is_dropped_and_the_request_retried` → `tests/clients_openai.rs::rejected_400_naming_param_drops_it_and_retries_once`, `tests/clients_openai.rs::learned_params_persist_on_instance`
- `test_other_openai_rejection_wordings_are_recognised` → `tests/clients_openai.rs::other_rejection_wordings_are_recognised`, `tests/clients_golden.rs::rejected_parameter_matches_python`
- `test_rejected_token_limit_is_swapped_not_dropped` → `tests/clients_openai.rs::rejected_token_limit_is_swapped_not_dropped`
- `test_400_without_a_named_parameter_is_still_a_config_error` → `tests/clients_openai.rs::bad_request_without_named_param_is_config_error_once`
- `test_rejection_of_a_parameter_we_did_not_send_is_not_retried_forever` → `tests/clients_openai.rs::rejected_param_not_sent_is_not_retried`
- `test_list_models_openai_compat_returns_sorted_ids` → `tests/clients_openai.rs::models_sorted`, `tests/clients_openai.rs::list_models_goes_through_the_built_client`
- `test_list_models_missing_endpoint_maps_to_no_model_list` → `tests/clients_openai.rs::models_404_is_no_model_list`
- `test_list_models_response_without_data_maps_to_no_model_list` → `tests/clients_openai.rs::models_without_data_array_is_no_model_list`
- `test_list_models_auth_error_maps_to_config_error` → `tests/clients_openai.rs::models_auth_errors_are_config`
- `test_list_models_connection_error_maps_to_offline` → `tests/clients_openai.rs::models_connection_failure_is_offline`
- `test_list_models_retryable_status_maps_to_offline` → `tests/clients_openai.rs::models_retryable_statuses_are_offline`
- `test_list_models_claude_uses_models_api` → `tests/clients_claude.rs::claude_models_paginate`
- `test_list_models_claude_status_error_mapping` → `tests/clients_claude.rs::claude_models_status_mapping`
- `test_list_models_claude_connection_error_maps_to_offline` → `tests/clients_claude.rs::claude_list_models_connection_error_maps_to_offline`
- `test_error_detail_extracts_api_message` → `tests/clients_golden.rs::error_detail_matches_python`
- `test_error_detail_keeps_the_whole_message` → `tests/clients_golden.rs::error_detail_matches_python`（golden 含超長多行訊息案例）
- `test_openai_compat_config_error_carries_api_message` → `tests/clients_openai.rs::status_4xx_is_config_with_detail`
- `test_openai_compat_other_4xx_maps_to_config_error` → `tests/clients_openai.rs::bad_request_without_named_param_is_config_error_once`
- `test_openai_compat_offline_status_carries_api_message` → `tests/clients_openai.rs::status_429_and_5xx_are_offline`
- `test_openai_compat_connection_error_carries_reason` → `tests/clients_openai.rs::connection_failure_is_offline_without_status`（只斷言原因非空且不含金鑰；Python 釘的是 httpx 的 `[Errno 11001] getaddrinfo failed` 字樣，Rust 的原因字串來自 reqwest／系統，字樣不同，已知差異）
- `test_claude_status_error_carries_api_message` → `tests/clients_claude.rs::claude_status_error_carries_api_message`
- `test_claude_other_4xx_maps_to_config_error` → `tests/clients_claude.rs::claude_retry_conditions_follow_the_sdk`
- `test_list_models_config_error_carries_api_message` → `tests/clients_openai.rs::models_error_statuses_map_like_chat`
- `test_token_limit_rejected_under_both_names_raises_instead_of_looping` → `tests/clients_openai.rs::token_limit_rejected_under_both_names_stops`
- `test_openai_compat_asks_for_a_stream_and_joins_the_chunks` → `tests/clients_openai.rs::streams_and_joins_deltas`
- `test_only_the_official_openai_endpoint_asks_for_usage_in_the_stream` → `tests/clients_openai.rs::official_uses_max_completion_tokens_and_no_temperature`, `tests/clients_openai.rs::custom_keeps_max_tokens_and_temperature_by_thinking_mode`
- `test_openai_compat_truncation_is_read_from_the_stream` → `tests/clients_openai.rs::length_finish_is_bad_output_with_sample`
- `test_cancelling_an_openai_compat_request_closes_the_stream` → `tests/clients_openai.rs::cancel_mid_stream_returns_cancelled_and_closes_connection`
- `test_a_request_cancelled_before_it_starts_is_never_sent` → `tests/clients_openai.rs::request_cancelled_before_start_is_never_sent`
- `test_a_connection_error_without_a_cancel_is_still_offline` → `tests/clients_openai.rs::stream_dropped_mid_response_is_offline`（本批新增）
- `test_the_request_handle_is_released_after_the_request_completes` → 不移植：Python 的 `RequestHandle` 要登記當下的回應物件、結束後撤銷，事後 `cancel()` 才不會去關已結束的回應；Rust 只傳 `CancellationToken` 並在請求期間 select 它，沒有登記的回應物件可誤關，請求結束後取消符記沒有任何作用
- `test_claude_uses_the_streaming_helper` → `tests/clients_claude.rs::claude_sends_structured_output_and_parses_translation`（斷言 body 的 `stream: true` 並解析 SSE；Python 的 `messages.stream` 輔助函式是 SDK 機制）
- `test_cancelling_a_claude_request_closes_the_stream` → `tests/clients_claude.rs::cancel_mid_stream_returns_cancelled`
- `test_a_request_on_a_closed_client_is_offline_so_the_pool_retries_it` → 不移植：Python 的 `reconfigure` 會關掉舊 client，競速時 httpx 對已關閉的 client 拋 `RuntimeError`，因此特地把它歸為離線；Rust 的 `reconfigure` 不關舊後端（飛行中的請求持有舊快照照常跑完，見 `Translator::reconfigure`），不會出現已關閉的 client（這也是行為差異：Python 的 `reconfigure` 關掉舊 client，仍在飛行中的請求會失敗；Rust 的該請求會以舊設定完成，因此舊設定翻出的譯文可能再回傳一次）
- `test_a_request_on_a_closed_claude_client_is_offline` → 不移植：同上（Anthropic SDK 的 client 被關閉後的行為），Rust 沒有已關閉的 client（這也是行為差異：Python 的 `reconfigure` 關掉舊 client，仍在飛行中的請求會失敗；Rust 的該請求會以舊設定完成，因此舊設定翻出的譯文可能再回傳一次）
- `test_an_unrelated_runtime_error_is_not_taken_for_offline` → 不移植：它只驗證上一組「已關閉 client」判斷不會把無關的 `RuntimeError` 誤當離線；Rust 沒有該判斷，錯誤皆由 reqwest／HTTP 狀態明確分類
- `test_gemini_joins_streamed_chunks` → `tests/clients_gemini.rs::gemini_maps_roles_and_system_instruction`
- `test_gemini_sends_system_instruction_and_model_role` → `tests/clients_gemini.rs::gemini_maps_roles_and_system_instruction`
- `test_gemini_low_effort_sends_thinking_level` → `tests/clients_gemini.rs::gemini_thinking_level_only_when_not_auto`
- `test_gemini_logs_prompt_cache_usage` → `tests/clients_gemini.rs::gemini_logs_prompt_cache_usage`
- `test_gemini_truncated_output_maps_to_bad_output` → `tests/clients_gemini.rs::gemini_max_tokens_finish_is_bad_output`
- `test_gemini_error_mapping` → `tests/clients_gemini.rs::gemini_error_mapping`, `tests/clients_gemini.rs::gemini_retry_behavior_matches_sdk`
- `test_gemini_cancel_closes_the_streamed_response` → `tests/clients_gemini.rs::cancel_mid_stream_returns_cancelled`
- `test_list_models_gemini_keeps_generate_content_models` → `tests/clients_gemini.rs::gemini_models_filtered_and_prefix_stripped`
- `test_list_models_gemini_error_mapping` → `tests/clients_gemini.rs::gemini_models_error_mapping`, `tests/clients_gemini.rs::gemini_list_models_connection_error_maps_to_offline`

## tests/translation/test_context.py

- `test_snapshot_returns_pushed_lines_in_order` → `src/translation/context.rs::snapshot_returns_pushed_lines_in_order`
- `test_snapshot_is_a_copy` → `src/translation/context.rs::snapshot_is_a_copy`
- `test_oldest_lines_drop_beyond_capacity` → `src/translation/context.rs::context_keeps_last_8_lines_per_tab`
- `test_concurrent_pushes_do_not_lose_lines` → `src/translation/context.rs::concurrent_pushes_do_not_lose_lines`
- `test_client_contexts_keep_tabs_apart` → `src/translation/context.rs::client_contexts_keep_tabs_apart`
- `test_client_contexts_current_follows_current_tab` → `src/translation/context.rs::client_contexts_current_follows_current_tab`

## tests/translation/test_examples.py

- `test_parse_generated_accepts_every_path` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_drops_only_the_failing_path` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_requires_the_original_in_parentheses` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_rejects_a_wrong_line_count` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_names_the_failed_region_condition` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_accepts_a_latin_script_target_with_translated_names` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_rejects_names_left_in_english_and_echoed_in_parentheses` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_tells_a_wrong_line_count_from_an_untranslated_line` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_accepts_an_interface_label_spelled_like_its_source` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_treats_a_missing_interface_label_as_untranslated` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_rejects_parentheses_on_the_interface_label` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_treats_a_line_echoing_its_source_as_missing` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_counts_a_numbered_preamble_as_a_failure` → `tests/examples_golden.rs::parse_matches_python`
- `test_parse_generated_ignores_a_preamble_and_code_fences` → `tests/examples_golden.rs::parse_matches_python`
- `test_merge_keeps_existing_paths_and_fills_missing_ones` → `tests/examples_golden.rs::merge_summary_and_digest`
- `test_summary_and_digest` → `tests/examples_golden.rs::merge_summary_and_digest`, `tests/examples_golden.rs::digest_matches_python`
- `test_fingerprint_contains_service_target_and_revision_but_no_key` → `tests/examples_golden.rs::fingerprint_has_service_target_and_revision_but_no_key`
- `test_fingerprint_ignores_surrounding_whitespace_in_the_target` → `tests/examples_golden.rs::fingerprint_has_service_target_and_revision_but_no_key`（測試傳入 `" 日本語 "`，期望去空白後的指紋）
- `test_generation_request_names_the_target_and_numbers_the_sources` → `tests/examples_golden.rs::generation_request_names_the_target_and_numbers_the_sources`, `tests/examples_golden.rs::generation_request_matches_python`
- `test_game_language_examples_translate_into_english` → `tests/examples_golden.rs::game_language_examples_match_python`, `tests/examples_golden.rs::game_language_examples_are_complete_and_bare_label`
- `test_game_language_region_example_keeps_the_interface_label_bare` → `tests/examples_golden.rs::game_language_examples_are_complete_and_bare_label`
- `test_game_language_region_examples_have_no_surrounding_newlines` → `tests/examples_golden.rs::game_language_examples_are_complete_and_bare_label`
- `test_store_round_trips_and_keeps_several_fingerprints` → `tests/examples_golden.rs::store_round_trips_and_ignores_bad_files`
- `test_store_drops_the_oldest_beyond_the_limit` → `tests/examples_golden.rs::store_evicts_lru_beyond_32`
- `test_re_putting_a_fingerprint_moves_it_to_the_end` → `tests/examples_golden.rs::re_putting_moves_to_the_end`
- `test_a_failed_write_leaves_no_temp_file` → `tests/examples_golden.rs::a_failed_write_leaves_no_temp_file`（Rust 以「目標路徑是資料夾」製造寫入失敗，Python 以 monkeypatch `os.replace`）
- `test_store_rejects_a_pair_that_is_not_a_list_of_two_strings` → `tests/examples_golden.rs::store_round_trips_and_ignores_bad_files`
- `test_store_treats_a_corrupt_file_as_empty` → `tests/examples_golden.rs::store_round_trips_and_ignores_bad_files`
- `test_store_does_not_write_an_empty_set` → `tests/examples_golden.rs::store_does_not_write_an_empty_set`
- `test_cache_hit_applies_without_generating` → `tests/translator.rs::cache_hit_applies_without_generating`
- `test_a_miss_generates_stores_and_applies` → `tests/translator.rs::a_miss_generates_stores_and_applies`
- `test_two_translators_on_the_same_service_share_one_generation` → `tests/translator.rs::two_translators_on_the_same_service_share_one_generation`
- `test_a_failed_fingerprint_is_not_retried_in_the_same_run` → `tests/translator.rs::a_failed_fingerprint_is_not_retried_in_the_same_run`
- `test_an_empty_result_counts_as_a_failure` → `tests/translator.rs::an_empty_result_counts_as_a_failure`
- `test_a_result_for_an_outdated_fingerprint_is_stored_but_not_applied` → `tests/translator.rs::a_result_for_an_outdated_fingerprint_is_stored_but_not_applied`
- `test_a_result_is_applied_by_its_own_fingerprint_not_the_requested_one` → `tests/translator.rs::a_result_is_applied_by_its_own_fingerprint_not_the_requested_one`
- `test_on_applied_is_skipped_when_set_examples_rejects` → `tests/translator.rs::a_result_for_an_outdated_fingerprint_is_stored_but_not_applied`（該測試斷言 `set_examples` 因指紋過期而拒絕時 on_applied 為 0 次；Rust 的 `set_examples` 不能像 Python 換成假函式，只能用過期指紋觸發拒絕）
- `test_registering_the_same_translator_twice_registers_and_generates_once` → `tests/translator.rs::registering_the_same_translator_twice_registers_and_generates_once`
- `test_a_failure_after_the_settings_changed_does_not_block_a_later_retry` → `tests/translator.rs::a_failure_after_the_settings_changed_does_not_block_a_later_retry`
- `test_number_lines_skips_blank_lines` → `tests/postprocess_golden.rs::line_helpers_match_python`
- `test_unnumber_lines_accepts_various_number_styles_and_plain_output` → `tests/postprocess_golden.rs::numbering_matches_python`
- `test_numbered_entries_ignores_unnumbered_lines` → `tests/postprocess_golden.rs::numbering_matches_python`

## tests/translation/test_gate.py

- `test_allows_up_to_the_limit` → `tests/pool.rs::gate_allows_up_to_the_limit`
- `test_blocks_beyond_the_limit_until_released` → `tests/pool.rs::gate_blocks_beyond_the_limit_until_released`
- `test_stop_event_aborts_a_waiter` → `tests/pool.rs::shutdown_stops_waiting_acquires`
- `test_raising_the_limit_wakes_waiters` → `tests/pool.rs::gate_raising_the_limit_wakes_waiters`
- `test_lowering_the_limit_does_not_overcommit` → `tests/pool.rs::gate_lowering_the_limit_does_not_overcommit`

## tests/translation/test_pool.py

- `test_submitted_lines_are_translated_and_reported` → `tests/pool.rs::submitted_lines_are_translated_and_reported`
- `test_blocked_line_does_not_block_the_others` → `tests/pool.rs::blocked_line_does_not_block_the_others`
- `test_in_flight_counts_outstanding_work` → `tests/pool.rs::in_flight_counts_outstanding_work`
- `test_resize_keeps_pool_usable` → `tests/pool.rs::resize_changes_parallelism`
- `test_submit_survives_concurrent_resize` → `tests/pool.rs::submit_survives_concurrent_resize`
- `test_shutdown_cancels_queued_work_without_leaking_in_flight` → `tests/pool.rs::shutdown_cancels_queued_work_without_leaking_in_flight`
- `test_offline_retries_until_success_and_clears_error_state` → `tests/pool.rs::success_resets_backoff`
- `test_offline_sets_error_state_while_failing` → `tests/pool.rs::offline_backoff_5_15_30_then_stays_30`
- `test_error_detail_exposes_the_api_message_while_failing` → `tests/pool.rs::config_error_retries_every_15s`
- `test_error_detail_keeps_the_status_without_a_message_and_clears_after_recovery` → `tests/pool.rs::success_resets_backoff`, `tests/pool.rs::error_detail_is_none_without_status_or_message`
- `test_config_error_retries_and_reports_config_state` → `tests/pool.rs::config_error_retries_and_reports_config_state`, `tests/pool.rs::config_error_retries_every_15s`（前者為本批新增，補上設定錯誤恢復後狀態清除）
- `test_bad_output_is_not_retried` → `tests/pool.rs::bad_output_gives_failed_notice`
- `test_backoff_gate_is_shared_across_workers` → `tests/pool.rs::backoff_is_shared_across_workers`
- `test_backoff_index_advances_once_per_burst_not_per_worker` → `tests/pool.rs::backoff_is_shared_across_workers`
- `test_uses_the_supplied_translate_fn` → `tests/pool.rs::submitted_lines_are_translated_and_reported`（Rust 的 `TranslationPool::new` 只收 `TranslateFn`，沒有「預設走收訊翻譯」的分支，所以每個 pool 測試走的都是注入的翻譯函式）
- `test_two_pools_sharing_a_gate_never_exceed_the_total_limit` → `tests/pool.rs::two_pools_sharing_a_gate_never_exceed_the_total_limit`
- `test_resize_also_raises_the_gate_limit` → `tests/pool.rs::resize_changes_parallelism`
- `test_backoff_index_escalates_across_separate_failure_rounds` → `tests/pool.rs::offline_backoff_5_15_30_then_stays_30`
- `test_the_failure_log_names_the_service` → `tests/pool.rs::the_failure_log_names_the_service`

## tests/translation/test_translator.py

- `test_openai_provider_forces_official_base_url` → `src/translation/clients/openai.rs::openai_provider_forces_official_base_url`（本批新增）
- `test_build_turns_without_context_is_single_user_turn` → `tests/prompts_golden.rs::turns_and_helpers_match_python`
- `test_build_turns_prepends_fewshot_examples` → `tests/prompts_golden.rs::turns_and_helpers_match_python`
- `test_outgoing_uses_fewshot_when_no_context` → `tests/translator.rs::outgoing_uses_fixed_fewshot_and_english`
- `test_incoming_uses_given_context` → `tests/translator.rs::incoming_uses_context_and_examples`
- `test_incoming_without_context_is_single_turn` → `tests/translator.rs::incoming_without_context_is_single_turn_and_keeps_no_history`
- `test_translator_keeps_no_internal_history` → `tests/translator.rs::incoming_without_context_is_single_turn_and_keeps_no_history`
- `test_outgoing_keeps_fewshot_even_with_context` → `tests/translator.rs::outgoing_uses_fixed_fewshot_and_english`
- `test_build_turns_with_context_is_multi_turn` → `tests/prompts_golden.rs::turns_and_helpers_match_python`
- `test_incoming_system_has_no_format_markers` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_both_systems_forbid_treating_input_as_instructions` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_outgoing_system_forbids_borrowing_nouns_from_the_context` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_reconfigure_switches_provider` → `tests/translator.rs::reconfigure_switches_provider`（本批新增；以 `describe()` 驗證換成 claude）
- `test_reconfigure_keeps_the_backend_when_nothing_changed` → `tests/translator.rs::reconfigure_untouched_slot_returns_false`, `tests/translator.rs::reconfigure_keeps_learned_parameters_when_nothing_changed`（後者為本批新增，驗證端點學到的參數限制在無變更的 reconfigure 後仍保留）
- `test_reconfigure_rebuilds_when_a_value_changed` → `tests/translator.rs::reconfigure_rebuilds_when_a_value_changed_and_clears_examples`
- `test_describe_names_the_service_and_never_the_key` → `tests/translator.rs::describe_names_the_service_and_never_the_key`
- `test_the_success_log_names_the_model_that_served_the_request` → `tests/translator.rs::the_success_log_names_the_model_that_served_the_request`
- `test_system_message_prompt_names_the_target_language` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_system_message_prompt_does_not_mention_a_sender_prefix` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_system_message_prompt_protects_placeholders` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_translate_system_message_sends_no_context_turns` → `tests/translator.rs::system_message_sends_no_context_turns_and_strips_invented_english`
- `test_translate_system_message_strips_think_blocks` → `tests/clients_openai.rs::think_blocks_are_stripped`, `tests/clients_claude.rs::claude_sends_structured_output_and_parses_translation`（Rust 與 Python 一樣在 client 層剝除 think；translator 層的假 backend 不經過它）
- `test_game_noun_rule_is_shared_by_both_prompts` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_game_noun_rule_forbids_inventing_english_for_non_english_source` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_game_noun_rule_always_appends_the_original_for_every_target` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_system_prompts_no_longer_embed_an_example` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_example_turns_become_a_user_and_assistant_pair` → `tests/prompts_golden.rs::turns_and_helpers_match_python`
- `test_prompt_revision_was_bumped_for_the_example_change` → `tests/prompts_golden.rs::turns_and_helpers_match_python`（constants 案例釘 `prompt_revision`）
- `test_game_noun_rule_keeps_player_and_npc_names_untranslated` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_game_noun_rule_names_the_target_language` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_tidy_parentheses_removes_english_invented_for_a_cjk_source` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_english_copied_from_the_source` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_removes_english_the_source_never_had` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_ignores_an_english_sender_name` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_handles_full_width_parentheses` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_leaves_placeholders_alone` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_leaves_target_language_notes_alone` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_removes_every_invented_occurrence` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_removes_cjk_invented_for_a_latin_translation` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_cjk_copied_from_the_source_for_a_latin_translation` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_accented_notes_for_a_latin_translation` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_ignores_punctuation_when_matching_the_source` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_parentheses_the_player_typed` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_emoticons_built_from_parentheses` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_game_text_in_parentheses` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_a_line_that_is_only_parentheses` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_removes_an_original_that_only_echoes_the_name` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_tidy_parentheses_keeps_a_repeated_number_the_player_typed` → `tests/postprocess_golden.rs::tidy_parentheses_matches_python`
- `test_translate_incoming_strips_invented_english` → `tests/translator.rs::incoming_strips_invented_english`
- `test_translate_system_message_strips_invented_english` → `tests/translator.rs::system_message_sends_no_context_turns_and_strips_invented_english`
- `test_restore_sender_replaces_a_sender_the_model_rewrote` → `tests/postprocess_golden.rs::restore_sender_matches_python`, `tests/postprocess_golden.rs::sender_of_matches_python`
- `test_restore_sender_puts_back_a_sender_the_model_dropped` → `tests/postprocess_golden.rs::restore_sender_matches_python`, `tests/postprocess_golden.rs::sender_of_matches_python`
- `test_restore_sender_does_not_take_a_bracketed_message_for_the_sender` → `tests/postprocess_golden.rs::restore_sender_matches_python`, `tests/postprocess_golden.rs::sender_of_matches_python`
- `test_restore_sender_leaves_lines_without_a_sender_alone` → `tests/postprocess_golden.rs::restore_sender_matches_python`, `tests/postprocess_golden.rs::sender_of_matches_python`
- `test_translate_incoming_keeps_the_original_sender` → `tests/translator.rs::incoming_keeps_the_original_sender_and_logs_the_change`
- `test_only_whitespace_after_the_sender_is_not_logged_as_an_altered_sender` → `tests/translator.rs::only_whitespace_after_the_sender_is_not_logged_as_altered`
- `test_has_stray_latin_flags_a_translation_that_switched_to_english` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_flags_a_partly_englished_translation` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_accepts_a_translation_in_the_target_language` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_allows_english_the_source_already_had` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_is_off_for_a_latin_script_target_language` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_ignores_an_english_sender_name` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_looks_at_the_message_body_of_the_translation` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_system_message_prompt_demands_the_whole_translation_in_the_target_language` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_strict_system_message_prompt_calls_out_the_english_slip` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_translate_system_message_retries_when_the_model_answers_in_english` → `tests/translator.rs::system_message_strict_retry_on_stray_latin_only_once`
- `test_translate_system_message_does_not_retry_a_clean_translation` → `tests/translator.rs::system_message_does_not_retry_a_clean_translation`
- `test_translate_system_message_keeps_a_retry_that_is_still_english` → `tests/translator.rs::system_message_strict_retry_on_stray_latin_only_once`
- `test_incoming_translation_is_not_retried` → `tests/translator.rs::incoming_translation_is_not_retried`
- `test_translator_exposes_the_target_language` → `tests/translator.rs::new_builds_the_backend_for_the_provider`, `tests/translator.rs::reconfigure_rebuilds_when_a_value_changed_and_clears_examples`
- `test_reconfigure_updates_the_exposed_target_language` → `tests/translator.rs::reconfigure_rebuilds_when_a_value_changed_and_clears_examples`
- `test_has_stray_latin_flags_a_source_echoed_back_untranslated` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_accepts_a_translation_that_has_target_language_words` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_flags_an_english_sentence_built_around_a_player_name` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_allows_a_player_name_copied_from_the_source` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_ignores_case_and_spacing_when_matching_the_source` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_flags_a_name_the_model_swapped_in` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_covers_non_latin_target_languages` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_has_stray_latin_accepts_a_translation_without_any_latin` → `tests/postprocess_golden.rs::has_stray_latin_matches_python`
- `test_incoming_translation_logs_the_source_and_the_result` → `tests/translator.rs::incoming_logs_the_source_and_the_result`
- `test_outgoing_translation_logs_how_many_context_lines_it_carried` → `tests/translator.rs::outgoing_logs_how_many_context_lines_it_carried`
- `test_system_message_retry_is_logged_apart_from_the_first_attempt` → `tests/translator.rs::system_message_strict_retry_on_stray_latin_only_once`
- `test_prompts_drop_the_no_english_clauses_when_the_target_is_the_game_language` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_prompts_keep_the_no_english_clauses_for_other_latin_targets` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_incoming_slang_rule_keeps_abbreviations_when_the_target_is_the_game_language` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_game_language_match_ignores_case_and_spacing` → `tests/prompts_golden.rs::system_prompts_match_python`, `tests/prompts_golden.rs::turns_and_helpers_match_python`（`" english "` 的提示詞案例與 `is_game_language` 案例）
- `test_sender_prefix_limit_is_shared_with_the_chat_parser` → `tests/postprocess_golden.rs::sender_prefix_limit_is_shared_with_the_chat_parser`（本批新增）
- `test_region_text_sends_the_recognized_lines_without_numbers` → `tests/translator.rs::region_lines_roundtrip_without_numbers`
- `test_region_text_shows_merged_lines_as_a_whole_without_adding_the_source_back` → `tests/translator.rs::region_mismatched_count_returns_block`
- `test_region_text_drops_code_fences_around_the_translation` → `tests/translator.rs::region_drops_code_fences_around_the_translation`
- `test_region_text_with_no_translation_is_bad_output_not_an_empty_screen` → `tests/translator.rs::region_empty_is_bad_output`
- `test_region_text_strips_invented_english_from_merged_lines_too` → `tests/translator.rs::region_parentheses_are_judged_against_nearby_lines`
- `test_region_text_strips_english_the_model_invented_for_a_line_without_any` → `tests/translator.rs::region_parentheses_are_judged_against_nearby_lines`
- `test_region_text_keeps_only_english_copied_from_nearby_lines` → `tests/translator.rs::region_parentheses_are_judged_against_nearby_lines`
- `test_region_text_keeps_a_name_moved_into_the_adjacent_line` → `tests/translator.rs::region_parentheses_are_judged_against_nearby_lines`
- `test_region_text_strips_english_found_only_two_lines_away` → `tests/translator.rs::region_parentheses_are_judged_against_nearby_lines`
- `test_region_system_prompt_only_allows_parentheses_copied_from_the_line` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_region_system_prompt_asks_for_line_by_line_output_keeping_source_numbers` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_translate_region_text_logs_no_translation_content` → `tests/translator.rs::region_logs_no_translation_content`
- `test_region_system_prompt_carries_the_target_language_and_no_chat_format` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_region_system_prompt_speaks_of_recognized_text_not_of_an_image` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_region_system_prompt_pins_the_order_of_name_and_parenthesized_original` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_prompts_ask_to_translate_parentheses_already_in_the_source` → `tests/prompts_golden.rs::system_prompts_match_python`
- `test_game_noun_rule_treats_joined_camel_case_names_as_nouns_not_code` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_game_noun_rule_pins_the_order_of_name_and_original_for_every_prompt` → `tests/prompts_golden.rs::system_prompts_match_python`（整份提示詞逐 byte 比對；Rust 沒有獨立匯出的 `_game_noun_rule`，規則文字含在各提示詞內）
- `test_translate_incoming_sends_the_example_before_the_context` → `tests/translator.rs::incoming_uses_context_and_examples`
- `test_without_examples_no_example_turns_are_sent` → `tests/translator.rs::system_message_sends_no_context_turns_and_strips_invented_english`
- `test_region_text_sends_the_region_example_first` → `tests/translator.rs::region_sends_the_region_example_first`
- `test_set_examples_ignores_a_stale_fingerprint` → `tests/translator.rs::set_examples_ignores_a_stale_fingerprint`
- `test_reconfigure_clears_the_examples` → `tests/translator.rs::reconfigure_rebuilds_when_a_value_changed_and_clears_examples`
- `test_reconfigure_without_changes_keeps_the_examples` → `tests/translator.rs::reconfigure_untouched_slot_returns_false`
- `test_generate_examples_retries_bad_output_and_merges` → `tests/translator.rs::generate_examples_retries_bad_output_and_merges`
- `test_generate_examples_gives_up_after_six_requests` → `tests/translator.rs::generate_examples_gives_up_after_six_requests`
- `test_generate_examples_does_not_retry_a_config_error` → `tests/translator.rs::generate_examples_does_not_retry_other_errors`
- `test_generate_examples_for_the_game_language_sends_nothing` → `tests/translator.rs::english_target_uses_game_language_examples`
- `test_generate_and_store_reuses_a_cached_set_without_a_request` → `tests/translator.rs::generate_and_store_uses_the_cache_and_skips_empty_results`
- `test_test_translate_survives_a_failing_example_generation` → `tests/translator.rs::example_failure_after_a_good_test_still_returns_the_translation`
- `test_strict_retry_sends_the_same_example_turns` → `tests/translator.rs::strict_retry_sends_the_same_example_turns`
- `test_generate_and_store_persists_a_non_empty_result` → `tests/translator.rs::generate_and_store_uses_the_cache_and_skips_empty_results`
- `test_generate_and_store_skips_writing_an_empty_result` → `tests/translator.rs::generate_and_store_uses_the_cache_and_skips_empty_results`
- `test_a_reconfigure_mid_request_does_not_mix_settings_in_the_strict_retry` → `tests/translator.rs::a_reconfigure_mid_request_does_not_mix_settings_in_the_strict_retry`
- `test_a_reconfigure_after_the_prompt_is_built_still_uses_the_same_backend` → `tests/translator.rs::a_reconfigure_mid_request_keeps_the_same_backend_for_incoming_and_region`（本批新增；Python 在「提示詞建好之後、送出之前」插入 reconfigure，Rust 的 `translate_*` 在開頭取一次後端快照、中間沒有可插入的點，所以改在請求進行中 reconfigure，驗證請求仍用原後端與原提示詞）
- `test_generate_examples_returns_the_fingerprint_of_the_settings_it_used` → `tests/translator.rs::generate_examples_returns_the_fingerprint_of_the_settings_it_used`

## tests/ui/overlay/test_message_list.py

- `test_resize_updates_existing_message_wraplength` → `src/ui/overlay/message_list.rs::text_has_equal_side_margins_clear_of_the_scrollbar_and_resize_bands`, `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`
- `test_stick_to_bottom_when_view_at_bottom` → `src/ui/overlay/message_list.rs::stick_threshold_matches_python`
- `test_do_not_stick_when_scrolled_up` → `src/ui/overlay/message_list.rs::stick_threshold_matches_python`
- `test_add_message_appends_and_caps_at_max` → `src/ui/overlay/model.rs::max_messages_drops_oldest`
- `test_prune_removes_expired` → `src/ui/overlay/model.rs::fade_prunes_after_seconds_and_zero_disables`
- `test_no_fade_when_fade_seconds_zero` → `src/ui/overlay/model.rs::fade_prunes_after_seconds_and_zero_disables`
- `test_placeholder_centered_when_empty_hidden_after_message` → `src/ui/overlay/message_list.rs::placeholder_shown_when_empty`
- `test_update_message_fills_translation_in_place` → `src/ui/overlay/model.rs::update_fills_in_place_keeps_order`
- `test_pending_placeholder_uses_dimmer_colour_until_filled` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`
- `test_completed_message_is_not_dimmed` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`, `src/ui/overlay/message_list.rs::rows_are_painted_in_the_message_colours`
- `test_dimmed_scales_each_channel_toward_dark` → `src/ui/overlay/model.rs::dimmed_matches_python`
- `test_message_uses_game_color_translated_bright_original_dim` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`, `src/ui/overlay/message_list.rs::rows_are_painted_in_the_message_colours`
- `test_pending_message_restores_game_color_on_update` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`
- `test_message_without_color_falls_back_to_default_palette` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`
- `test_update_message_ignores_unknown_id` → `src/ui/overlay/model.rs::update_fills_hidden_view_and_ignores_unknown_id`
- `test_failed_translation_line_is_shown_in_error_colour` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`
- `test_successful_translation_keeps_the_game_colour` → `src/ui/overlay/model.rs::colors_follow_pending_failed_and_game_color`
- `test_narrowing_window_keeps_following_new_messages` → `src/ui/overlay/message_list.rs::narrowing_window_keeps_following_new_messages`
- `test_shortening_window_keeps_following_new_messages` → `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`（該測試的縮放序列含只縮矮視窗的一步）
- `test_error_banner_keeps_following_new_messages` → `src/ui/overlay/frame.rs::error_banner_keeps_following_new_messages`（新補）
- `test_scrolling_up_stops_following_until_back_at_bottom` → `src/ui/overlay/message_list.rs::does_not_follow_after_user_scrolls_up`
- `test_message_cap_keeps_scrolled_view_in_place` → `src/ui/overlay/message_list.rs::message_cap_keeps_scrolled_view_in_place`
- `test_prune_keeps_scrolled_view_in_place` → `src/ui/overlay/message_list.rs::anchor_keeps_reading_position_on_prune`
- `test_filling_a_pending_translation_keeps_scrolled_view_in_place` → `src/ui/overlay/message_list.rs::filling_a_pending_translation_above_keeps_scrolled_view_in_place`
- `test_prune_clears_a_selection_in_the_removed_row` → `src/ui/overlay/selection.rs::prune_of_selected_message_clears_selection`
- `test_max_messages_overflow_clears_a_selection_in_the_dropped_row` → `src/ui/overlay/selection.rs::max_messages_overflow_clears_a_selection_in_the_dropped_row`
- `test_update_message_clears_a_selection_in_that_row` → `src/ui/overlay/selection.rs::filling_a_selected_translation_clears_the_selection`
- `test_resize_redraws_the_highlight_to_the_new_wrapping` → `src/ui/overlay/selection.rs::selected_text_survives_scrolling_out_of_view_and_rewrapping`, `src/ui/text_select.rs::highlight_spans_every_visual_line_and_empty_ranges_have_none`（Rust 反白每幀依目前換行重算，沒有「重畫舊矩形」這一步）
- `test_prune_during_a_drag_keeps_the_selected_text` → `src/ui/overlay/message_list.rs::dragging_a_selection_suspends_auto_follow_until_content_changes`, `src/ui/overlay/selection.rs::prune_before_the_selection_keeps_the_text`
- `test_translation_fills_a_message_in_a_hidden_view` → `src/ui/overlay/model.rs::update_fills_hidden_view_and_ignores_unknown_id`
- `test_hidden_pending_and_failed_rows_keep_their_colours` → `src/ui/overlay/model.rs::hidden_pending_and_failed_rows_keep_their_colours`（新補）
- `test_max_messages_applies_per_tab` → `src/ui/overlay/model.rs::max_messages_applies_per_tab`
- `test_prune_drops_faded_messages_in_hidden_views_too` → `src/ui/overlay/model.rs::fade_prunes_after_seconds_and_zero_disables`
- `test_switching_views_clears_the_selection` → `src/ui/overlay/selection.rs::switching_views_clears_the_selection`
- `test_switching_views_returns_to_following_the_bottom` → `src/ui/overlay/message_list.rs::switching_views_returns_to_following_the_bottom`
- `test_layout_follows_the_old_frame_geometry` → `src/ui/overlay/message_list.rs::layout_follows_the_old_frame_geometry`
- `test_translation_growing_a_hidden_line_pushes_later_messages_down` → `src/ui/overlay/message_list.rs::layout_follows_the_old_frame_geometry`, `src/ui/overlay/model.rs::update_fills_hidden_view_and_ignores_unknown_id`
- `test_resize_while_hidden_rewraps_on_show` → `src/ui/overlay/message_list.rs::resize_while_hidden_rewraps_on_show`（新補）
- `test_pruning_above_a_selection_keeps_text_and_highlight` → `src/ui/overlay/selection.rs::prune_before_the_selection_keeps_the_text`
- `test_wheel_scrolls_a_newly_created_view` → `src/ui/overlay/message_list.rs::wheel_scrolls_a_newly_created_view`
- `test_switching_views_keeps_each_views_canvas` → 不移植：Tk 專屬：每個分頁一個 canvas 控件並以 `pack` 掛卸；Rust 單一清單每幀依目前 view 的訊息繪製，沒有控件可保留。切回分頁的可見行為由 `switching_views_returns_to_following_the_bottom` 守住
- `test_rewrap_between_taking_and_restoring_the_anchor_keeps_the_view` → `src/ui/overlay/message_list.rs::reading_history_keeps_the_viewport_top_message_in_place_on_resize`（Python 守的是 `update_idletasks` 夾在取錨點與還原之間的競態，Rust 沒有這個中間事件；同一個使用者可見結果（縮放後閱讀位置不動）仍由該測試守住）
- `test_resizing_with_a_selection_does_not_grow_the_font_cache` → 不移植：Python 專屬：逐行 Tcl 字型物件的快取洩漏；Rust 沒有逐行字型物件，字型取自 egui 字型表
- `test_add_after_a_tab_switch_in_the_same_turn_still_sticks_to_bottom` → `src/ui/overlay/message_list.rs::add_after_a_tab_switch_in_the_same_turn_still_sticks_to_bottom`（新補）
- `test_a_burst_of_adds_keeps_a_scrolled_up_view_in_place` → `src/ui/overlay/message_list.rs::does_not_follow_after_user_scrolls_up`
- `test_showing_a_stale_tab_rewraps_it_before_it_is_visible` → `src/ui/overlay/message_list.rs::showing_a_stale_tab_rewraps_it_before_it_is_visible`（新補）
- `test_resize_draws_only_the_viewport_then_completes_identically` → `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`（斷言縮放後只排視口內的列、且全部用新換行寬度；Rust 沒有「先排視口、再背景補完」的兩階段，所以沒有「補完後與全部重畫相同」要比對（規格 F「Rust 版只要達到同樣的不卡頓即可」））
- `test_wheeling_up_after_a_resize_never_shows_a_gap` → `src/ui/overlay/message_list.rs::wheeling_up_after_a_resize_never_shows_a_gap`
- `test_dragging_the_scrollbar_to_the_top_lands_on_the_first_message` → `src/ui/overlay/message_list.rs::dragging_the_scrollbar_to_the_top_lands_on_the_first_message`
- `test_taller_window_during_partial_layout_shows_no_gap` → `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`（該測試的縮放序列含視窗變高，每步都斷言 `covered`（視口無空白）；「部分排版」階段 Rust 不存在）
- `test_resizing_an_empty_view_logs_nothing` → 不移植：Python 以 monkeypatch 把慢速門檻降為 −1 來強制觸發分段重排 log；Rust 只在實際換行且耗時 ≥50 ms 時記一行 `view relaid out`（`SLOW_RELAYOUT_MS`），沒有可注入門檻的鉤子，空清單提早返回、到不了該 log。已知未測：空視圖不記 log 靠程式碼路徑，沒有自動化斷言
- `test_a_slow_viewport_fill_is_logged_but_not_inside_a_relayout` → 不移植：Tk 專屬：分批補畫（viewport fill）機制；Rust 沒有補畫階段，不產生該 log
- `test_follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize` → `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`
- `test_reading_history_keeps_the_viewport_top_message_in_place_on_resize` → `src/ui/overlay/message_list.rs::reading_history_keeps_the_viewport_top_message_in_place_on_resize`
- `test_updates_trims_and_adds_during_partial_layout_match_a_full_redraw` → 不移植：Tk 專屬：部分排版／回填狀態下更新、裁剪、新增要與完整重畫一致；Rust 每幀由訊息與寬度重算視口，沒有可與完整重畫分歧的中間狀態。更新、裁剪、新增後的位置分別由 `layout_follows_the_old_frame_geometry`、`message_cap_keeps_scrolled_view_in_place`、`filling_a_pending_translation_above_keeps_scrolled_view_in_place` 守住
- `test_dragging_the_scrollbar_to_the_bottom_lands_on_the_newest_message` → `src/ui/overlay/message_list.rs::dragging_the_scrollbar_to_the_bottom_lands_on_the_newest_message`
- `test_dragging_the_scrollbar_to_the_top_lands_on_the_first_message_when_estimates_are_low` → `src/ui/overlay/message_list.rs::dragging_the_scrollbar_to_the_top_lands_on_the_first_message`（該測試先變窄讓上方估計高度偏低再拖到頂）
- `test_pruning_the_whole_drawn_range_during_partial_layout_matches_a_full_redraw` → `src/ui/overlay/message_list.rs::pruning_the_rows_in_view_lands_on_the_oldest_remaining_message`
- `test_background_backfill_completes_without_further_input` → 不移植：Tk 專屬：背景分批補畫；規格 F 僅要求達到同樣的不卡頓，Rust 只排視口內的列，沒有補畫
- `test_only_the_last_width_of_a_drag_is_backfilled` → 不移植：Tk 專屬：拖拉縮放時取消／重排補畫計時器；Rust 沒有補畫
- `test_backfill_batches_yield_to_idle_between_them` → 不移植：Tk 專屬：補畫批次與 idle 佇列的讓出；Rust 沒有補畫
- `test_a_hidden_view_is_not_backfilled_and_resumes_when_shown` → 不移植：Tk 專屬：隱藏分頁暫停補畫、顯示後續補；Rust 沒有補畫，隱藏分頁不排版、顯示時依當前寬度排版（見 `resize_while_hidden_rewraps_on_show`）
- `test_switching_back_to_a_view_whose_newest_message_is_undrawn_relays_it_out` → `src/ui/overlay/message_list.rs::switching_views_returns_to_following_the_bottom`
- `test_destroying_the_overlay_cancels_its_pending_work` → 不移植：Tk 專屬：已排的 `after` 工作在視窗銷毀後呼叫已刪除的 Tcl 命令；Rust 沒有排程中的補畫工作

## tests/ui/overlay/test_outlined_text.py

- `test_draw_outlined_matches_the_old_per_line_canvas` → `src/ui/overlay/outlined.rs::outline_copies_first_then_the_fill_on_top`
- `test_line_height_matches_the_old_canvas_height` → `src/ui/overlay/outlined.rs::line_height_adds_origin_outline_and_margin`
- `test_draw_outlined_at_an_offset_moves_every_item` → `src/ui/overlay/outlined.rs::outline_copies_first_then_the_fill_on_top`（該測試以行原點 (0, 50) 驗證九份文字的座標）
- `test_line_items_are_exactly_that_lines_text_items` → 不移植：Tk 專屬：canvas item id 與 tag 的對應；egui 即時繪製，沒有 item 可列舉
- `test_measured_height_matches_the_drawn_line` → 不移植：Python 專屬：`measure_height` 為了不建立 canvas item 而與繪製分開量測；Rust 排版一次得到 `Galley`，高度與繪製用同一份，沒有兩條路徑需要比對

## tests/ui/overlay/test_perf.py

- `test_switching_tabs_is_independent_of_message_count` → `src/ui/overlay/message_list.rs::switching_tabs_is_independent_of_message_count`（新補；以「每幀排版列數」取代 CPU 時間門檻（時間門檻在共用機器上不穩）：兩分頁各 1000 則，來回切換每幀都只排視口內不到 60 列）
- `test_adding_a_message_stays_cheap` → `src/ui/overlay/message_list.rs::adding_a_message_stays_cheap`（新補；同樣以每幀排版列數取代平均耗時門檻）
- `test_the_synchronous_part_of_a_resize_is_independent_of_message_count` → `src/ui/overlay/message_list.rs::relayout_1000_messages_within_budget`, `src/ui/overlay/message_list.rs::follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize`（前者是 `#[ignore]` 的 release 限定耗時門檻（CI 另跑 `cargo test --release -- --ignored relayout`），後者斷言縮放後只排視口內的列）
- `test_each_backfill_batch_stays_short` → 不移植：Tk 專屬：背景補畫批次；Rust 沒有補畫

## tests/ui/overlay/test_selection.py

- `test_line_text_reads_the_foreground_item` → 不移植：Tk 專屬：從 canvas 的前景 item 讀文字；Rust 選取直接用訊息文字與 `Galley`
- `test_visual_lines_cover_the_whole_text` → `src/ui/text_select.rs::highlight_spans_every_visual_line_and_empty_ranges_have_none`
- `test_visual_line_starts_agree_with_tk` → 不移植：Tk 專屬：視覺行首索引是向 Tk 的 text index 探測出來的；Rust 的視覺行直接取自 `Galley.rows`，沒有第二條探測路徑
- `test_highlight_x_agrees_with_tk_hit_test` → `src/ui/text_select.rs::highlight_x_agrees_with_hit_test`
- `test_highlight_spans_every_visual_line` → `src/ui/text_select.rs::highlight_spans_every_visual_line_and_empty_ranges_have_none`
- `test_empty_selection_has_no_rects` → `src/ui/text_select.rs::highlight_spans_every_visual_line_and_empty_ranges_have_none`
- `test_selected_text_within_one_line` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_selected_text_across_both_lines` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_selected_text_of_an_empty_span` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_selected_text_includes_whole_lines_in_between` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_selected_text_separates_messages_with_a_blank_line` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_selected_text_of_two_whole_messages` → `src/ui/overlay/selection.rs::selected_text_matches_python`
- `test_begin_on_a_line_reports_a_hit` → `src/ui/overlay/selection.rs::press_on_a_line_starts_a_selection_and_takes_focus`
- `test_begin_outside_every_message_reports_no_hit` → `src/ui/overlay/selection.rs::press_outside_every_message_starts_nothing`
- `test_begin_far_to_the_right_of_a_line_reports_no_hit` → `src/ui/overlay/selection.rs::press_outside_every_message_starts_nothing`（涵蓋視口右側（捲軸區）按下不起手；兩軸都要命中由 `hit_testing_follows_the_scroll_position` 補）
- `test_a_bare_press_is_not_a_selection` → `src/ui/overlay/selection.rs::a_bare_press_is_not_a_selection`
- `test_dragging_across_both_lines_selects_both` → `src/ui/overlay/selection.rs::dragging_across_both_lines_selects_both`
- `test_reverse_drag_selects_the_same_text` → `src/ui/overlay/selection.rs::reverse_drag_normalizes_span`
- `test_dragging_below_the_message_clamps_to_its_end` → `src/ui/overlay/selection.rs::dragging_past_the_messages_clamps_to_their_ends`
- `test_dragging_above_the_message_clamps_to_its_start` → `src/ui/overlay/selection.rs::dragging_past_the_messages_clamps_to_their_ends`
- `test_selection_spans_messages_with_a_blank_line_between` → `src/ui/overlay/selection.rs::selection_spans_messages`
- `test_selection_spans_three_messages` → `src/ui/overlay/selection.rs::selection_spans_three_messages_and_highlights_every_line_under_its_text`
- `test_dragging_below_everything_clamps_to_the_last_message` → `src/ui/overlay/selection.rs::dragging_past_the_messages_clamps_to_their_ends`
- `test_dragging_above_everything_clamps_to_the_first_message` → `src/ui/overlay/selection.rs::dragging_past_the_messages_clamps_to_their_ends`
- `test_highlight_is_drawn_on_every_spanned_message` → `src/ui/overlay/selection.rs::selection_spans_three_messages_and_highlights_every_line_under_its_text`
- `test_forgetting_a_row_before_the_selection_keeps_the_text` → `src/ui/overlay/selection.rs::prune_before_the_selection_keeps_the_text`
- `test_forgetting_a_row_inside_the_selection_clears_it` → `src/ui/overlay/selection.rs::prune_of_selected_message_clears_selection`
- `test_text_of_a_span_over_an_undrawn_row_comes_from_its_texts` → `src/ui/overlay/selection.rs::selected_text_survives_scrolling_out_of_view_and_rewrapping`
- `test_hit_testing_skips_undrawn_rows` → `src/ui/overlay/selection.rs::rows_scrolled_out_of_view_are_not_hit`
- `test_paint_adds_the_highlight_of_one_row_only` → 不移植：Tk 專屬：逐列重繪（`paint(row)`）；Rust 每幀依可見列整批繪製高亮，每行一塊且壓在該行文字之下由 `selection_spans_three_messages_and_highlights_every_line_under_its_text` 守住
- `test_dragging_above_everything_clamps_to_the_first_drawn_row` → `src/ui/overlay/selection.rs::dragging_above_the_viewport_scrolls_and_grows_the_selection`（Rust 沒有「未畫列」，拖到視口上方會自動捲動並延伸選取到更早的訊息）
- `test_paint_on_an_undrawn_row_draws_nothing` → 不移植：Tk 專屬：對尚未畫出的列呼叫 `paint`；Rust 只對本幀排版出的列產生高亮，不存在未畫列的繪製呼叫
- `test_highlight_is_drawn_below_the_text` → `src/ui/overlay/selection.rs::selection_spans_three_messages_and_highlights_every_line_under_its_text`
- `test_clear_removes_the_highlight_and_the_selection` → `src/ui/overlay/selection.rs::clear_removes_the_highlight_and_the_selection`
- `test_forget_clears_a_selection_in_that_row` → `src/ui/overlay/selection.rs::prune_of_selected_message_clears_selection`
- `test_redraw_keeps_the_selected_text` → `src/ui/overlay/selection.rs::selected_text_survives_scrolling_out_of_view_and_rewrapping`
- `test_hit_accounts_for_canvas_scroll` → `src/ui/overlay/selection.rs::hit_testing_follows_the_scroll_position`

## tests/ui/overlay/test_window.py

- `test_is_click_within_threshold` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_is_click_beyond_threshold_is_drag` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_minimize_counts_unread_and_expand_resets` → `src/ui/app.rs::minimize_counts_unread_and_expand_resets`, `src/ui/overlay/model.rs::unread_counts_once_per_broadcast_only_for_shown_client`
- `test_moved_to_adds_delta` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_south_east_grows_without_moving` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_south_east_clamps_to_minimum` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_west_moves_origin_and_keeps_right_edge` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_north_moves_origin_and_keeps_bottom_edge` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_west_freezes_origin_at_minimum_width` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_resized_edge_north_west_resizes_both_axes` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_edge_at_outside_window_is_no_edge` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_edge_at_interior_is_no_edge` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_edge_at_each_side` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_edge_at_corners_take_priority_over_sides` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_edge_at_ignores_corner_band_away_from_the_other_axis` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_error_banner_toggle` → `src/ui/overlay/banners.rs::error_and_update_banners_coexist_above_the_list`
- `test_explicit_position_and_size_applied` → `src/ui/overlay/frame.rs::initial_placement_waits_for_the_window_then_applies_once`, `src/ui/overlay/frame.rs::first_launch_is_centered_and_saved_geometry_is_kept`
- `test_geometry_change_callback_fires_on_manual_apply` → `src/ui/overlay/frame.rs::release_after_drag_reports_geometry_once`
- `test_set_status_updates_bar_label` → `src/ui/overlay/frame.rs::title_bar_items_are_vertically_centred_and_buttons_are_bar_tall_squares`, `tests/overlay_relabel.rs::status_key_relabels_on_language_change`（前者驗證標題列顯示 `status.listening` 文字，後者驗證換狀態／語言後重繪）
- `test_thumb_hidden_when_content_fits` → `src/ui/overlay/scrollbar.rs::thumb_span_matches_python`
- `test_thumb_span_follows_view_fraction` → `src/ui/overlay/scrollbar.rs::thumb_span_matches_python`
- `test_thumb_span_keeps_minimum_length_inside_track` → `src/ui/overlay/scrollbar.rs::thumb_span_matches_python`
- `test_scrollbar_stops_above_resize_grip` → `src/ui/overlay/message_list.rs::scrollbar_stays_clear_of_the_resize_bands_and_grip`
- `test_scroll_fraction_subtracts_grab_offset_and_clamps` → `src/ui/overlay/scrollbar.rs::scroll_fraction_matches_python`
- `test_point_in_rect_edges` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_should_auto_expand_when_bubble_becomes_foreground` → `src/ui/overlay/bubble.rs::auto_expand_only_on_becoming_foreground_without_pointer`
- `test_no_auto_expand_while_cursor_on_bubble` → `src/ui/overlay/bubble.rs::auto_expand_only_on_becoming_foreground_without_pointer`, `src/ui/overlay/bubble.rs::holding_the_bubble_blocks_auto_expand_even_off_the_rect`
- `test_no_auto_expand_when_bubble_already_foreground` → `src/ui/overlay/bubble.rs::auto_expand_only_on_becoming_foreground_without_pointer`
- `test_no_auto_expand_for_other_windows` → `src/ui/overlay/bubble.rs::auto_expand_only_on_becoming_foreground_without_pointer`
- `test_minimize_starts_foreground_watch_and_expand_stops_it` → `src/ui/app.rs::foreground_poll_runs_only_while_minimized`
- `test_bubble_release_without_press_is_ignored` → `src/ui/overlay/bubble.rs::release_without_press_is_ignored`
- `test_expand_reanchors_view_to_bottom` → `src/ui/app.rs::expand_reanchors_view_to_bottom`（新補）
- `test_messages_received_as_a_bubble_are_visible_after_expand` → `src/ui/app.rs::messages_received_as_a_bubble_are_visible_after_expand`（新補）
- `test_refresh_labels_retranslates_status_and_banner` → `tests/overlay_relabel.rs::status_key_relabels_on_language_change`
- `test_message_font_follows_language` → `tests/overlay_relabel.rs::status_key_relabels_on_language_change`（換語言後一般與粗體字族的主字型檔跟著換（zh-CN → msyh））
- `test_window_close_button_runs_the_clean_shutdown` → `src/ui/app.rs::close_request_runs_on_close_once_before_the_window_goes`
- `test_bubble_close_button_runs_the_clean_shutdown` → `src/ui/app.rs::bubble_close_takes_the_root_close_path`, `src/ui/overlay/bubble.rs::close_request_takes_the_shutdown_path`
- `test_set_update_shows_a_clickable_banner` → `src/ui/overlay/banners.rs::clicking_the_update_row_opens_the_release_page`, `src/ui/overlay/banners.rs::dismissed_update_version_not_reoffered`
- `test_update_banner_uses_an_opaque_background` → 不移植：Tk 專屬：視窗以 `-transparentcolor` 色鍵挖空，與底色同色的像素連 hit-test 都跳過；Rust 是逐像素 alpha，沒有色鍵。整列與 ✕ 可點的使用者可見行為由 `clicking_the_update_row_opens_the_release_page` 與 `dismissed_update_version_not_reoffered` 守住
- `test_update_banner_leaves_room_for_the_resize_grip` → `src/ui/overlay/banners.rs::update_row_leaves_room_for_the_resize_grip`
- `test_error_banner_formats_the_api_message_and_survives_language_switch` → `src/ui/overlay/banners.rs::error_banner_formats_args_and_links_urls`, `tests/overlay_relabel.rs::status_key_relabels_on_language_change`
- `test_error_banner_makes_urls_in_the_api_message_clickable` → `src/ui/overlay/banners.rs::error_banner_formats_args_and_links_urls`, `src/ui/overlay/banners.rs::error_banner_link_yields_to_the_left_resize_band`
- `test_update_banner_and_error_banner_coexist` → `src/ui/overlay/banners.rs::error_and_update_banners_coexist_above_the_list`
- `test_update_banner_follows_language_and_width` → `tests/overlay_relabel.rs::status_key_relabels_on_language_change`, `src/ui/overlay/banners.rs::update_row_hit_area_stays_inside_the_visible_strip`（語言由前者驗證；窄寬度換行由後者以長文字在窄視窗驗證，Rust 無 Tk `wraplength` 可斷言）
- `test_every_status_has_both_a_label_and_a_colour` → `src/ui/overlay/frame.rs::every_status_has_a_colour`
- `test_wrapped_banners_stay_left_aligned` → 不移植：Tk 專屬：`tk.Label` 多行預設置中、要設 `justify="left"`；egui 標籤預設靠左，沒有這項設定可斷言
- `test_refresh_labels_keeps_the_title_free_of_the_old_menu_glyph` → `tests/overlay_relabel.rs::status_key_relabels_on_language_change`, `src/ui/overlay/frame.rs::title_bar_items_are_vertically_centred_and_buttons_are_bar_tall_squares`（前者斷言換語言後視窗標題仍是 app 名稱；標題列文字就是傳入的名稱，Rust 沒有 ≡ 字元的來源）
- `test_minimize_clears_the_selection` → `src/ui/app.rs::minimizing_clears_the_selection`
- `test_press_on_a_message_starts_a_selection` → `src/ui/overlay/selection.rs::press_on_a_line_starts_a_selection_and_takes_focus`, `src/ui/overlay/selection.rs::dragging_across_both_lines_selects_both`
- `test_press_on_a_row_scrolled_out_of_view_is_ignored` → `src/ui/overlay/selection.rs::rows_scrolled_out_of_view_are_not_hit`
- `test_backdrop_press_away_from_any_edge_starts_a_selection` → `src/ui/overlay/selection.rs::dragging_from_the_first_character_selects_instead_of_resizing`（Rust 沒有底窗／畫布兩層事件路徑，只有一條；起手位置貼近縮放帶但不在帶內時仍選取）
- `test_backdrop_press_on_an_edge_still_resizes` → `src/ui/overlay/selection.rs::press_in_a_resize_band_resizes_instead_of_selecting`
- `test_view_does_not_jump_to_the_bottom_while_selecting` → `src/ui/overlay/message_list.rs::dragging_a_selection_suspends_auto_follow_until_content_changes`, `src/ui/overlay/message_list.rs::a_message_arriving_in_the_release_frame_does_not_snap_to_the_bottom`
- `test_copy_writes_only_when_something_is_selected` → `src/ui/overlay/selection.rs::ctrl_c_with_caps_lock_copies`
- `test_right_click_without_a_selection_pops_no_menu` → `src/ui/overlay/selection.rs::context_menu_only_with_selection`
- `test_right_click_with_a_selection_pops_the_themed_menu` → `src/ui/overlay/selection.rs::context_menu_only_with_selection`
- `test_choosing_copy_from_the_menu_copies_and_closes` → `src/ui/overlay/selection.rs::context_menu_only_with_selection`
- `test_clicking_a_message_closes_the_menu` → `src/ui/overlay/selection.rs::pressing_anywhere_else_closes_the_menu`
- `test_clicking_the_title_bar_closes_the_menu` → `src/ui/overlay/selection.rs::pressing_anywhere_else_closes_the_menu`
- `test_autoscroll_is_still_inside_the_viewport` → `src/ui/text_select.rs::autoscroll_pixels_matches_python`
- `test_autoscroll_goes_up_above_the_viewport_and_down_below_it` → `src/ui/text_select.rs::autoscroll_pixels_matches_python`
- `test_autoscroll_speeds_up_with_distance_but_is_capped` → `src/ui/text_select.rs::autoscroll_pixels_matches_python`
- `test_dragging_above_the_viewport_scrolls_and_grows_the_selection` → `src/ui/overlay/selection.rs::dragging_above_the_viewport_scrolls_and_grows_the_selection`
- `test_autoscroll_stops_once_the_drag_ends` → `src/ui/overlay/selection.rs::dragging_above_the_viewport_scrolls_and_grows_the_selection`（同一測試的後半段斷言放開後不再捲動）
- `test_minimize_closes_the_menu` → `src/ui/app.rs::minimizing_closes_the_copy_menu`
- `test_selection_entry_points_are_bound` → 不移植：Tk 專屬：檢查 `bind` 是否掛上事件序列；Rust 沒有綁定表，入口行為由會真的送出事件的測試覆蓋（`ctrl_c_with_caps_lock_copies`、`context_menu_only_with_selection`、`press_on_a_line_starts_a_selection_and_takes_focus`）
- `test_offer_update_shows_the_banner_when_nothing_was_dismissed` → `src/ui/overlay/banners.rs::clear_update_does_not_count_as_dismissing`, `src/ui/overlay/banners.rs::offer_update_skips_versions_not_newer_than_dismissed`
- `test_offer_update_skips_a_version_the_user_dismissed` → `src/ui/overlay/banners.rs::dismissed_update_version_not_reoffered`, `src/ui/overlay/banners.rs::offer_update_skips_versions_not_newer_than_dismissed`
- `test_set_update_ignores_the_dismissed_version` → `src/ui/overlay/banners.rs::dismissed_update_version_not_reoffered`
- `test_region_button_invokes_the_callback_when_provided` → `src/ui/overlay/frame.rs::each_title_button_maps_to_its_action_without_moving`
- `test_region_button_absent_without_a_callback` → 不移植：Rust 的 ⛶ 按鈕一律存在（`Clicked::Region` 由宿主處理），沒有「未提供回呼就隱藏」的分支；正式程式兩邊都固定提供，使用者可見行為一致。已知：此分支在 Rust 不存在，不是測試缺口
- `test_region_button_tooltip_shows_the_configured_hotkey` → `src/ui/overlay/frame.rs::region_tooltip_shows_the_hotkey_after_a_pause`
- `test_set_region_hotkey_updates_the_tooltip_on_the_next_show` → `src/ui/app.rs::region_tooltip_shows_the_set_hotkey_after_half_a_second`
- `test_clear_update_does_not_count_as_dismissing` → `src/ui/overlay/banners.rs::clear_update_does_not_count_as_dismissing`
- `test_tab_bar_shows_game_labels_and_highlights_current` → `src/ui/overlay/tab_bar.rs::shows_every_game_label`, `src/ui/overlay/tab_bar.rs::only_the_current_tab_gets_a_two_pixel_accent_underline`, `src/ui/app.rs::tab_bar_sits_between_the_title_bar_and_the_list`
- `test_tab_bar_hidden_until_a_state_arrives` → `src/ui/overlay/tab_bar.rs::hidden_without_tabs`
- `test_messages_are_kept_per_tab_and_follow_the_game_tab` → `src/ui/overlay/model.rs::messages_go_to_each_tab_view`
- `test_messages_for_a_tab_not_in_the_bar_are_kept` → `src/ui/overlay/model.rs::messages_go_to_each_tab_view`
- `test_broadcast_lands_in_every_listed_tab` → `src/ui/overlay/model.rs::broadcast_lands_in_every_listed_tab`
- `test_foreground_slot_switches_the_whole_view` → `src/ui/overlay/model.rs::foreground_slot_switches_the_whole_view`
- `test_repeating_the_foreground_slot_keeps_the_view` → `src/ui/overlay/message_list.rs::repeating_the_foreground_slot_keeps_the_view`, `src/ui/events.rs::repeated_foreground_slot_neither_queues_nor_repaints`（前者新補）
- `test_unread_counts_only_the_foreground_client` → `src/ui/overlay/model.rs::unread_counts_once_per_broadcast_only_for_shown_client`
- `test_autoscrolling_a_selection_into_the_undrawn_area_keeps_it_on_the_same_text` → `src/ui/overlay/selection.rs::selected_text_survives_scrolling_out_of_view_and_rewrapping`, `src/ui/overlay/selection.rs::dragging_above_the_viewport_scrolls_and_grows_the_selection`

## tests/ui/test_fonts.py

- `test_font_family_follows_language` → `tests/i18n_golden.rs::font_family_matches_python`, `src/ui/fonts_egui.rs::primary_font_first_then_other_cjk_then_thai_and_symbols`
- `test_font_family_comes_from_the_script` → `tests/i18n_golden.rs::font_family_matches_python`
- `test_font_accepts_weight` → `src/ui/fonts_egui.rs::bold_file_follows_the_primary_font`, `src/ui/fonts_egui.rs::system_fonts_put_bold_and_icon_files_first`

## tests/ui/test_icons.py

- `test_apply_window_icon_survives_missing_file` → 不移植：Rust 的圖示以 `include_bytes!` 編進 exe（規格「單一 exe 與資源」），沒有可能遺失的 .ico 檔；解碼失敗是程式錯誤，由 `app_icon()` 的 `expect` 直接擋下
- `test_apply_window_icon_installs_it_on_the_window_class` → `src/ui/app.rs::root_viewport_carries_the_app_icon`, `tests/app_main.rs::exe_embeds_the_app_icon_resource`（Rust 在根 viewport 與設定視窗各自 `with_icon`，沒有 Win32 視窗類別圖示可讀；輸入框、框選等無工作列的子視窗不帶圖示）

## tests/ui/test_input_box.py

- `test_stale_session_discarded_on_cancel` → `src/ui/input_box.rs::stale_session_discarded_on_cancel`
- `test_error_message_urls_are_clickable` → `src/ui/input_box.rs::error_message_urls_are_clickable`
- `test_finish_over_limit_keeps_window_and_blocks_send` → `src/ui/input_box.rs::finish_over_limit_keeps_window_and_blocks_send`
- `test_finish_within_limit_sends_and_closes` → `src/ui/input_box.rs::finish_within_limit_sends_and_closes`
- `test_close_with_stale_target_hwnd_does_not_crash` → `src/ui/input_box.rs::close_with_stale_target_hwnd_does_not_crash`（新補；同時斷言 `win32::force_foreground` 對已不存在的 hwnd 回 false 而不 panic）
- `test_enter_on_empty_input_closes_window` → `src/ui/input_box.rs::enter_on_empty_input_closes_window`
- `test_non_cancelled_translation_succeeds` → `src/ui/input_box.rs::non_cancelled_translation_succeeds`
- `test_stale_error_discarded_on_cancel` → `src/ui/input_box.rs::stale_error_discarded_on_cancel`
- `test_current_error_shown_on_error` → `src/ui/input_box.rs::current_error_shown_on_error`
- `test_worker_failure_error_callback_runs` → `tests/app_main.rs::outgoing_translate_posts_the_localized_error_and_logs_the_service`, `src/ui/input_box.rs::current_error_shown_on_error`（Python 的 `NameError` 是 except 區塊變數出範圍的 Python 專屬 bug；Rust 驗證失敗確實以 `UiEvent::OutgoingDone` 送回並顯示）
- `test_anchored_position_sits_below_the_anchor` → `src/ui/input_box.rs::anchored_position_sits_below_the_anchor`
- `test_anchored_position_flips_above_when_no_room_below` → `src/ui/input_box.rs::anchored_position_flips_above_when_no_room_below`
- `test_anchored_position_clamps_into_the_work_area` → `src/ui/input_box.rs::anchored_position_clamps_into_the_work_area`
- `test_show_places_the_box_below_the_anchor` → `src/ui/input_box.rs::show_uses_the_anchor_width`（驗證錨點正下方、左緣對齊、與錨點同寬；Python 要扣 Tk 視窗的隱形邊框（寬 −2、左 −7），Rust 以 `win32::remove_system_frame` 去掉系統外框，外框矩形就是可見矩形，無須扣除）
- `test_show_flips_above_using_the_visible_frame_height` → `src/ui/input_box.rs::box_placed_above_the_anchor_grows_upward`, `src/ui/input_box.rs::anchored_position_flips_above_when_no_room_below`（Rust 以含標題條的整體內容高度算翻到上方的位置，沒有 Tk 的「client 高 + 標題列」換算）
- `test_show_without_anchor_sits_at_the_cursor` → `src/ui/input_box.rs::show_without_anchor_sits_at_the_cursor`
- `test_clear_anchor_makes_show_use_the_cursor` → `src/ui/input_box.rs::clear_anchor_makes_show_use_the_cursor`
- `test_anchor_set_while_open_applies_on_the_next_show` → `src/ui/input_box.rs::anchor_set_while_open_applies_on_the_next_show`
- `test_show_uses_the_anchor_width` → `src/ui/input_box.rs::show_uses_the_anchor_width`, `src/ui/input_box.rs::width_follows_the_anchor_in_physical_pixels_at_150_percent`
- `test_shown_box_visible_frame_matches_the_anchor` → `src/ui/input_box.rs::show_uses_the_anchor_width`, `src/ui/input_box.rs::width_follows_the_anchor_in_physical_pixels_at_150_percent`（Python 以 DWM 讀回實際可見邊界；Rust 因無系統外框，矩形運算即可見矩形，實際視窗擺放（`set_window_rect`）靠實機驗收）
- `test_growth_stays_pinned_when_tk_already_grew_the_window` → `src/ui/input_box.rs::box_placed_above_the_anchor_grows_upward`, `src/ui/input_box.rs::growth_after_dragging_pins_the_dragged_bottom`（「Tk 先自行撐高再補 `_fit_height`」的競態在 Rust 不存在：`refit` 是以量到的內容高度與目前外框一次算出的純函式）
- `test_growth_after_dragging_pins_the_dragged_bottom` → `src/ui/input_box.rs::growth_after_dragging_pins_the_dragged_bottom`
- `test_box_placed_above_the_anchor_grows_upward` → `src/ui/input_box.rs::box_placed_above_the_anchor_grows_upward`
- `test_visible_chrome_measures_the_frame_of_a_hidden_window` → 不移植：Rust 輸入框用 `win32::remove_system_frame` 去掉系統外框與標題列（規格允許外觀改變），沒有需要量測的隱形邊框與標題列；擺放仍用同一份錨點定位語意（`open_rect`）
- `test_show_clamps_the_anchor_width_to_minimum` → `src/ui/input_box.rs::show_clamps_the_anchor_width_to_minimum`
- `test_show_without_any_anchor_uses_the_default_width` → `src/ui/input_box.rs::show_without_any_anchor_uses_the_default_width`
- `test_bottom_pin_does_not_loop_when_the_geometry_is_rewritten` → 不移植：Tk 專屬：`-Y` 幾何寫法在 `<Configure>` 回呼裡重入的無限迴圈；Rust 沒有事件回呼重入，`input_host.rs` 只在量到的內容高度改變時才套用一次 `refit`
- `test_fit_height_shrinks_when_the_status_needs_fewer_lines` → `src/ui/input_host.rs::fit_height_shrinks_when_the_status_needs_fewer_lines`（新補）
- `test_shown_box_fits_the_wrapped_hint` → `src/ui/input_host.rs::shown_box_fits_the_wrapped_hint`（新補；斷言提示在窄寬度換行且內容高度涵蓋整段提示）
- `test_box_is_not_resizable` → 不移植：Tk 視窗組態（`resizable()`）；Rust 輸入框無系統外框，只設 `ViewportBuilder::with_resizable(false)`，沒有可拖的縮放邊，亦無可斷言的狀態
- `test_error_message_follows_language` → `tests/app_main.rs::outgoing_translate_posts_the_localized_error_and_logs_the_service`（錯誤文案由 `t("input.failed")` 在失敗當下現取；各語系字串由 i18n golden 與語言檔檢查把關，該測試以 `t()` 取期望值、未固定為英文字面）
- `test_is_open_tracks_window_lifecycle` → `src/ui/input_box.rs::is_open_tracks_window_lifecycle`
- `test_outgoing_translation_failure_is_logged` → `tests/app_main.rs::outgoing_translate_posts_the_localized_error_and_logs_the_service`
- `test_retarget_changes_target_hwnd_while_open` → `src/ui/input_box.rs::retarget_changes_target_hwnd_while_open`, `tests/input_box.rs::retarget_logs_only_real_changes`
- `test_retarget_is_a_no_op_for_none_or_same_hwnd` → `src/ui/input_box.rs::retarget_is_a_no_op_for_none_or_same_hwnd`, `tests/input_box.rs::retarget_logs_only_real_changes`
- `test_hide_keeps_the_unsent_draft_for_the_next_show` → `src/ui/input_box.rs::hide_keeps_the_unsent_draft_for_the_next_show`, `src/ui/input_box.rs::restored_draft_puts_the_cursor_at_the_end`
- `test_manual_close_discards_the_draft` → `src/ui/input_box.rs::manual_close_discards_the_draft`
- `test_sent_text_is_not_restored` → `src/ui/input_box.rs::sent_text_is_not_restored`
- `test_hide_while_translating_restores_an_editable_draft` → `src/ui/input_box.rs::hide_while_translating_restores_an_editable_draft`
- `test_closing_the_box_while_translating_cancels_the_request` → `src/ui/input_box.rs::closing_the_box_while_translating_cancels_the_request`
- `test_the_failure_log_names_the_service` → `tests/app_main.rs::outgoing_translate_posts_the_localized_error_and_logs_the_service`

## tests/ui/test_popup.py

- `test_clamped_position_keeps_a_fitting_popup_where_it_is` → `tests/geometry_golden.rs::geometry_matches_python`（`clamped_position` 的 golden 案例）
- `test_clamped_position_pulls_back_from_the_right_edge` → `tests/geometry_golden.rs::geometry_matches_python`（`clamped_position` 的 golden 案例）
- `test_clamped_position_pulls_back_from_the_bottom_edge` → `tests/geometry_golden.rs::geometry_matches_python`（`clamped_position` 的 golden 案例）
- `test_clamped_position_never_goes_past_the_top_left` → `tests/geometry_golden.rs::geometry_matches_python`（`clamped_position` 的 golden 案例）
- `test_show_makes_it_visible_with_the_given_label` → `src/ui/overlay/selection.rs::context_menu_only_with_selection`
- `test_label_follows_what_is_passed_each_time` → `tests/overlay_relabel.rs::status_key_relabels_on_language_change`（換語言後選單文字由「複製」變 "Copy"）
- `test_hide_makes_it_invisible` → `src/ui/overlay/selection.rs::pressing_anywhere_else_closes_the_menu`
- `test_clicking_the_row_fires_the_callback_and_closes` → `src/ui/overlay/selection.rs::context_menu_only_with_selection`
- `test_the_popup_never_takes_keyboard_focus` → 不移植：Rust 選單是 overlay 同一 viewport 內的 `egui::Area`（`src/ui/copy_menu.rs`），不是獨立視窗，沒有焦點可被取走；選單開著時 Ctrl+C 仍有效由 `src/ui/overlay/selection.rs::ctrl_c_copies_while_the_copy_menu_is_open` 守住（新補）
- `test_hover_swaps_the_row_colours` → `src/ui/overlay/selection.rs::hovering_the_menu_swaps_its_colours`
- `test_hiding_an_already_hidden_popup_is_harmless` → 不移植：Rust 選單沒有 `hide()`，開關由 `menu_rect: Option` 表示，關閉就是設為 `None`，不存在「對已隱藏者再隱藏」的呼叫
- `test_the_popup_never_becomes_the_active_window` → 不移植：Tk 獨立 Toplevel 的 `WS_EX_NOACTIVATE` 樣式；Rust 選單與 overlay 同一視窗、不是另一個可被啟用的視窗
- `test_the_popup_is_a_borderless_topmost_window` → 不移植：Tk 獨立 Toplevel 的 overrideredirect／topmost 屬性；Rust 選單以 `Order::Foreground` 疊在 overlay 之上，位置不超出視窗由 `src/ui/overlay/selection.rs::menu_stays_inside_the_overlay` 守住

## tests/ui/test_richtext.py

- `test_linkify_wraps_bare_urls_in_link_markup` → `tests/richtext_golden.rs::linkify_matches_python`
- `test_rich_label_shows_plain_text_and_reports_it_back` → `src/ui/richtext.rs::plain_text_has_no_links`
- `test_rich_label_turns_bare_urls_into_link_segments` → `tests/richtext_golden.rs::linkify_matches_python`, `tests/richtext_golden.rs::link_markup_matches_python`, `src/ui/richtext.rs::clicking_a_bare_url_opens_it`
- `test_rich_label_grows_to_fit_wrapped_lines` → `src/ui/richtext.rs::long_text_wraps_within_the_width`
- `test_rich_label_click_on_link_opens_browser` → `src/ui/richtext.rs::clicking_a_bare_url_opens_it`
- `test_rich_label_fits_a_long_unbreakable_word` → `src/ui/richtext.rs::rich_label_fits_a_long_unbreakable_word`（新補；Python 守的是 Tk `count displaylines` 少算一行，Rust 守同一個使用者可見結果：超長無空白字串後的文字仍完整畫在區域內）
- `test_rich_label_reports_height_changes` → `src/ui/input_host.rs::fit_height_shrinks_when_the_status_needs_fewer_lines`, `src/ui/input_host.rs::shown_box_fits_the_wrapped_hint`（Rust 沒有高度變動回呼，`content()` 每幀回傳內容高度給宿主）
- `test_rich_label_set_blocks_joins_blocks_with_a_blank_line` → `src/ui/region/card.rs::source_sits_dimmed_and_smaller_above_the_translation_after_a_blank_line`
- `test_rich_label_set_blocks_colours_each_block` → `src/ui/region/card.rs::source_sits_dimmed_and_smaller_above_the_translation_after_a_blank_line`
- `test_rich_label_set_blocks_keeps_links_clickable` → `src/ui/region/card.rs::urls_are_underlined_links_and_markup_shows_only_its_text`, `src/ui/region/card.rs::clicking_a_link_opens_it_and_closes_the_card`
- `test_rich_label_set_before_layout_never_balloons` → 不移植：Tk 專屬：先 `set` 再 `pack` 時寬度還是佔位的 1px，長句被算成一字一行；egui 每幀以實際可用寬度排版，沒有佔位寬度階段
- `test_rich_label_natural_width_is_the_widest_line_unwrapped` → `src/ui/region/card.rs::rich_label_natural_width_is_the_widest_line_unwrapped`（新補；`natural_width` 在 Rust 屬框選結果卡）
- `test_rich_label_natural_width_measures_each_block_with_its_own_font` → `src/ui/region/card.rs::rich_label_natural_width_measures_each_block_with_its_own_font`（新補）
- `test_rich_label_natural_width_counts_the_link_label_not_the_markup` → `src/ui/region/card.rs::rich_label_natural_width_counts_the_link_label_not_the_markup`（新補）

## tests/ui/test_tooltip.py

- `test_hovering_shows_the_tooltip_after_a_delay` → `src/ui/overlay/frame.rs::region_tooltip_shows_the_hotkey_after_a_pause`, `src/ui/app.rs::region_tooltip_shows_the_set_hotkey_after_half_a_second`
- `test_leaving_hides_the_tooltip` → `src/ui/overlay/frame.rs::leaving_hides_the_tooltip`（新補）
- `test_leaving_before_the_delay_elapses_cancels_the_show` → `src/ui/overlay/frame.rs::leaving_before_the_delay_elapses_cancels_the_show`（新補）
- `test_text_fn_is_re_evaluated_on_each_show` → `src/ui/app.rs::region_tooltip_shows_the_set_hotkey_after_half_a_second`, `tests/overlay_relabel.rs::status_key_relabels_on_language_change`（提示文字每幀現取（`t()` 加目前熱鍵），換熱鍵、換語言後下一次顯示即更新）
- `test_button_press_cancels_and_hides` → `src/ui/overlay/frame.rs::button_press_hides_the_tooltip`（新補；原本 Rust 按住按鈕時提示不會收起，已修（見 `fix(rust)` 提交）；放開後游標仍停在按鈕上時，egui 會再顯示提示，舊版要離開再進入才會再現）
- `test_tooltip_never_takes_keyboard_focus` → 不移植：Rust 提示框是 egui 在同一 viewport 內畫的浮層（`on_hover_ui_at_pointer`），不是獨立視窗，沒有焦點可取

## tests/ui/region/test_box.py

- `test_show_opens_a_window_around_the_rect_with_room_for_the_handles` → `src/ui/region/region_box.rs::window_rect_extends_the_rect_by_the_margin_on_each_side`
- `test_dragging_a_corner_handle_resizes_and_reports_the_new_rect` → `src/ui/region/region_box.rs::dragging_an_edge_resizes_only_that_edge`
- `test_dragging_an_edge_line_moves_only_that_edge` → `src/ui/region/region_box.rs::dragging_an_edge_resizes_only_that_edge`
- `test_dragging_inside_the_box_moves_it` → `src/ui/region/region_box.rs::dragging_the_inside_moves_the_rect`, `src/ui/region_host.rs::box_follows_its_rect_on_screen_coordinates`
- `test_the_mouse_layer_is_nearly_invisible_and_covers_the_whole_box` → 不移植：Tk 專屬：Tk 靠另一片幾乎全透明（alpha 不超過 0.02）的視窗蓋住整個框來接滑鼠；Rust 只有一個透明 viewport，整個視窗本來就收得到滑鼠（見 `src/ui/region/region_box.rs` 檔頭），沒有第二層視窗可斷言
- `test_dragging_an_edge_on_the_mouse_layer_resizes_too` → `src/ui/region/region_box.rs::hit_at_reports_edges_corners_and_move`, `src/ui/region/region_box.rs::dragging_an_edge_resizes_only_that_edge`（Rust 只有一條滑鼠路徑；邊帶最內側一排仍算邊由 `hit_at_reports_edges_corners_and_move` 守住）
- `test_a_click_on_the_band_changes_nothing` → `src/ui/region/region_box.rs::a_click_restores_the_rect_and_reports_nothing`
- `test_the_opposite_corner_stays_put_and_the_size_never_drops_below_the_minimum` → `src/ui/region/region_box.rs::resizing_stops_at_the_minimum_size`
- `test_the_window_follows_the_box_while_dragging` → `src/ui/region_host.rs::box_follows_its_rect_on_screen_coordinates`, `src/ui/region/region_box.rs::a_click_restores_the_rect_and_reports_nothing`（拖曳中 `RegionBox::drag` 回傳新矩形、宿主 `show_box` 每幀依它搬視窗；搬視窗那一步（`set_window_rect`）需要真實視窗，沒有自動測試，以實機驗證）
- `test_only_the_line_and_the_handles_are_painted` → `src/ui/region/region_box.rs::only_the_line_and_the_handles_are_painted`（新補；Python 以色鍵透明色與畫布項目顏色斷言，Rust 斷言畫出的矩形只有一條框線與八個把手、全是同一個亮色）
- `test_show_while_open_moves_the_existing_box` → 不移植：Rust 的框以 `RegionBox::new(rect)` 建立，重新框選會先收掉舊框（`src/ui/region/flow.rs::a_new_selection_hides_the_previous_box`）再開新的，沒有「開著時再 show 換位置」這條路徑；框跟著矩形走由 `src/ui/region_host.rs::box_follows_its_rect_on_screen_coordinates` 與宿主每幀 `place` 負責
- `test_hide_is_idempotent` → 不移植：Rust 的框是否開著由 flow 持有的 `Option<RegionBox>` 表示，關閉就是設為 `None`，沒有 `hide()` 可重複呼叫
- `test_hit_at_tells_corners_from_edges_from_the_inside` → `src/ui/region/region_box.rs::hit_at_reports_edges_corners_and_move`

## tests/ui/region/test_card.py

- `test_pending_then_text` → `src/ui/region/card.rs::pending_shows_the_working_notice_in_the_pending_colour`, `src/ui/region/card.rs::text_replaces_the_pending_notice_in_the_translated_colour`
- `test_show_text_with_source_puts_both_in_one_selectable_label` → `src/ui/region/card.rs::source_sits_dimmed_and_smaller_above_the_translation_after_a_blank_line`, `src/ui/region/card.rs::dragging_past_the_text_selects_to_its_end`
- `test_show_text_without_source_shows_only_the_translation` → `src/ui/region/card.rs::text_without_source_shows_only_the_translation`
- `test_empty_text_shows_the_no_text_notice` → `src/ui/region/card.rs::empty_text_shows_the_no_text_notice`
- `test_error_is_red` → `src/ui/region/card.rs::error_is_red`
- `test_click_anywhere_hides` → `src/ui/region/card.rs::a_click_closes_and_a_drag_does_not`
- `test_close_affordance_is_shown` → `src/ui/region/card.rs::close_affordances_are_shown_apart_from_the_content`
- `test_close_button_tooltip_shows_close_text` → `src/ui/region/card.rs::close_button_tooltip_says_close`, `src/ui/region/card.rs::close_button_press_hides_the_tooltip`（後者新補：Tk 的提示按下即收，Rust 原本按住 ✕ 時提示不收，已修，見 `fix(rust)` 提交）
- `test_close_button_hides` → `src/ui/region/card.rs::close_button_closes_on_press`
- `test_hide_is_idempotent` → 不移植：Rust 的卡片沒有 `hide()`；開著與否由 flow 持有的 `Option<Card>` 表示，關閉就是設為 `None`
- `test_card_is_anchored_below_the_rect_with_the_rect_width` → `src/ui/region/card.rs::card_sits_below_the_rect_with_its_width`
- `test_card_below_the_rect_is_at_least_the_minimum_height` → `src/ui/region/card.rs::card_sits_below_the_rect_with_its_width`
- `test_side_card_takes_the_text_width_instead_of_the_rect_width` → `src/ui/region/card.rs::card_moves_to_the_wider_side_when_neither_fits`
- `test_side_card_stays_beside_the_rect_after_rewrapping_wider` → `src/ui/region/card.rs::side_placement_sticks_until_the_content_changes`
- `test_card_is_capped_to_the_work_area_and_scrolls_when_neither_side_fits` → `src/ui/region/card.rs::capped_card_scrolls_with_the_wheel_and_shows_a_thin_scrollbar`
- `test_wheel_scrolls_the_capped_card` → `src/ui/region/card.rs::capped_card_scrolls_with_the_wheel_and_shows_a_thin_scrollbar`
- `test_clicking_the_scrollbar_does_not_close_the_card` → `src/ui/region/card.rs::clicking_the_scrollbar_keeps_the_card_and_the_selection`
- `test_narrow_rect_gets_the_minimum_width` → `src/ui/region/card.rs::card_sits_below_the_rect_with_its_width`
- `test_right_click_on_translation_shows_the_copy_menu` → `src/ui/region/card.rs::right_click_menu_copies_selection_or_everything`, `src/ui/region/card.rs::right_click_does_not_close_the_card`
- `test_right_click_does_not_close_the_card` → `src/ui/region/card.rs::right_click_does_not_close_the_card`
- `test_click_without_drag_closes_the_card` → `src/ui/region/card.rs::a_click_closes_and_a_drag_does_not`
- `test_drag_on_the_translation_label_selects_instead_of_closing` → `src/ui/region/card.rs::a_click_closes_and_a_drag_does_not`, `src/ui/region/card.rs::drag_selection_asks_to_borrow_focus`
- `test_copy_selection_and_menu_copy_share_the_clipboard` → `src/ui/region/card.rs::right_click_menu_copies_selection_or_everything`, `src/ui/region/card.rs::ctrl_c_copies_the_selection_through_our_clipboard`, `src/ui/region/card.rs::dragging_past_the_text_selects_to_its_end`, `src/ui/region/card.rs::a_collapsed_selection_is_no_selection`
- `test_focus_for_copy_swallows_a_tcl_error_when_the_window_is_gone` → 不移植：Tk 專屬：視窗已被銷毀時 `focus_force` 會丟 `TclError`；Rust 的卡片只回報 `CardAction::FocusForCopy`，由宿主決定怎麼搶焦點，沒有可丟例外的 Tk 呼叫
- `test_control_c_entry_points_are_bound` → `src/ui/region/card.rs::ctrl_c_copies_the_selection_through_our_clipboard`（Rust 直接送出 Ctrl+C 事件驗證複製結果，不是檢查綁定）
- `test_selection_stays_visible_without_keyboard_focus` → `src/ui/region/card.rs::drag_selection_asks_to_borrow_focus`（egui 的選取反白是卡片自己畫的，與鍵盤焦點無關，沒有 Tk 的 `inactiveselectbackground` 設定可斷言；測試確認拖曳後有畫出反白）
- `test_closing_by_click_or_the_close_button_notifies_on_close` → `src/ui/region/card.rs::a_click_closes_and_a_drag_does_not`, `src/ui/region/card.rs::close_button_closes_on_press`, `src/ui/region/flow.rs::closing_the_card_hides_the_box`（Rust 以 `CardAction::Close` 回報，不用 `on_close` 回呼）
- `test_replacing_the_card_from_the_flow_does_not_count_as_closing` → 不移植：Rust 的卡片沒有 `on_close` 回呼；`CardAction::Close` 只在使用者點卡片或 ✕ 時產生，flow 自己換卡片或收卡片不會經過這條路
- `test_the_card_leaves_room_for_the_box_handles_below_the_rect` → `src/ui/region/card.rs::the_card_leaves_room_for_the_box_handles_below_the_rect`（新補）
- `test_geometry_sits_below_when_it_fits` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_below_and_above_floor_the_height_at_the_minimum` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_flips_above_when_no_room_below` → `tests/geometry_golden.rs::geometry_matches_python`, `src/ui/region/card.rs::card_flips_above_when_no_room_below`
- `test_geometry_moves_to_the_wider_side_when_neither_above_nor_below_fits` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_beside_the_rect_uses_the_text_width` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_beside_the_rect_caps_the_width_to_the_side_room` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_shrinks_to_the_side_room_but_not_below_the_minimum_width` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_side_minimum_width_is_wider_than_the_rect_minimum` → `src/ui/region/card.rs::side_minimum_width_is_wider_than_the_rect_minimum`（新補）
- `test_geometry_beside_the_rect_slides_up_to_stay_inside_the_work_area` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_caps_the_height_to_the_work_area` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_geometry_keeps_the_minimum_height_on_a_tiny_work_area` → `tests/geometry_golden.rs::geometry_matches_python`
- `test_beside_geometry_matches_the_side_branch_of_anchored_geometry` → `tests/geometry_golden.rs::geometry_matches_python`

## tests/ui/region/test_flow.py

- `test_toggle_opens_the_selector_and_a_second_toggle_cancels_it` → `src/ui/region/flow.rs::toggle_opens_the_selector_and_a_second_toggle_cancels_it`
- `test_toggle_passes_the_captured_frame_to_the_selector` → `src/ui/region/flow.rs::toggle_passes_the_captured_frame_to_the_selector`
- `test_toggle_passes_the_captured_backdrop_to_the_selector` → `src/ui/region/flow.rs::toggle_passes_the_captured_backdrop_to_the_selector`
- `test_selection_captures_and_fills_the_card_with_the_translation` → `src/ui/region/flow.rs::selection_captures_and_fills_the_card_with_the_translation`
- `test_selection_passes_the_source_text_to_the_card` → `src/ui/region/flow.rs::selection_passes_the_source_text_to_the_card`
- `test_selection_crops_the_same_frame_that_was_captured` → `src/ui/region/flow.rs::selection_crops_the_same_frame_that_was_captured`
- `test_frame_capture_failure_is_shown_at_the_cursor_without_opening_the_selector` → `src/ui/region/flow.rs::frame_capture_failure_is_shown_at_the_cursor_without_opening_the_selector`
- `test_the_selector_covers_the_monitor_under_the_middle_of_the_game_client_area` → `src/ui/region/flow.rs::the_selector_covers_the_monitor_under_the_middle_of_the_game_client_area`
- `test_capture_failure_is_shown_on_the_card_without_a_worker` → `src/ui/region/flow.rs::capture_failure_is_shown_on_the_card_without_a_worker`
- `test_plain_capture_error_is_shown_as_capture_failed` → `src/ui/region/flow.rs::plain_capture_error_is_shown_as_capture_failed`
- `test_pipeline_errors_are_described_on_the_card` → `src/ui/region/flow.rs::pipeline_errors_are_described_on_the_card`
- `test_stale_result_is_dropped_after_a_new_selection` → `src/ui/region/flow.rs::stale_result_is_dropped_after_a_new_selection`
- `test_stale_error_is_dropped_after_a_new_selection` → `src/ui/region/flow.rs::stale_error_is_dropped_after_a_new_selection`
- `test_selection_moves_foreground_before_showing_the_pending_card` → `src/ui/region/flow.rs::selection_moves_foreground_before_showing_the_pending_card`
- `test_toggle_hides_a_previous_card` → `src/ui/region/flow.rs::toggle_hides_a_previous_card`
- `test_esc_style_cancel_restores_the_games_foreground` → `src/ui/region/flow.rs::esc_style_cancel_restores_the_games_foreground`
- `test_hotkey_cancel_restores_the_games_foreground` → `src/ui/region/flow.rs::hotkey_cancel_restores_the_games_foreground`
- `test_toggle_with_hwnd_zero_is_ignored_once_the_selector_is_already_closed` → `src/ui/region/flow.rs::toggle_with_hwnd_zero_is_ignored_once_the_selector_is_already_closed`
- `test_set_alpha_forwards_to_the_card` → `src/ui/region/flow.rs::set_alpha_forwards_to_the_card`
- `test_start_from_button_opens_the_selector_with_the_found_window` → `src/ui/region/flow.rs::start_from_button_opens_the_selector_with_the_found_window`
- `test_start_from_button_shows_an_error_card_when_no_game_window_is_found` → `src/ui/region/flow.rs::start_from_button_shows_an_error_card_when_no_game_window_is_found`
- `test_start_from_button_cancels_an_already_open_selector` → `src/ui/region/flow.rs::start_from_button_cancels_an_already_open_selector`
- `test_describe_error_maps_each_failure_kind` → `tests/region_pipeline.rs::describe_error_maps_each_failure_kind`
- `test_selection_leaves_the_box_on_the_selected_rect` → `src/ui/region/flow.rs::selection_leaves_the_box_on_the_selected_rect`
- `test_adjusting_the_box_recaptures_the_live_game_and_retranslates` → `src/ui/region/flow.rs::adjusting_the_box_recaptures_the_live_game_and_retranslates`
- `test_capture_failure_while_adjusting_is_shown_and_keeps_the_box` → `src/ui/region/flow.rs::capture_failure_while_adjusting_is_shown_and_keeps_the_box`
- `test_closing_the_card_hides_the_box` → `src/ui/region/flow.rs::closing_the_card_hides_the_box`
- `test_a_new_selection_hides_the_previous_box` → `src/ui/region/flow.rs::a_new_selection_hides_the_previous_box`
- `test_each_request_gets_its_own_cancel_handle` → `src/ui/region/flow.rs::each_request_gets_its_own_cancel_handle`
- `test_adjusting_the_box_cancels_the_previous_request` → `src/ui/region/flow.rs::adjusting_the_box_cancels_the_previous_request`
- `test_a_new_selection_cancels_the_previous_request` → `src/ui/region/flow.rs::a_new_selection_cancels_the_previous_request`
- `test_closing_the_card_cancels_the_request` → `src/ui/region/flow.rs::closing_the_card_cancels_the_request`
- `test_a_cancelled_request_leaves_the_card_alone` → `tests/app_main.rs::region_runner_cancel_posts_nothing`
- `test_the_failure_log_names_the_service` → `tests/app_main.rs::region_runner_failure_logs_the_service`

## tests/ui/region/test_select.py

- `test_drag_reports_a_normalized_rect_in_screen_coordinates` → `src/ui/region/selector.rs::drag_reports_the_screen_rect`, `src/ui/region/selector.rs::drag_reports_physical_pixels_on_scaled_displays`
- `test_a_click_without_drag_cancels` → `src/ui/region/selector.rs::a_click_without_drag_cancels`
- `test_escape_cancels` → `src/ui/region/selector.rs::escape_cancels`
- `test_show_while_open_replaces_the_previous_layer` → `src/ui/region_host.rs::selector_closing_starts_a_fresh_window_for_the_next_selection`, `src/ui/region/flow.rs::toggle_opens_the_selector_and_a_second_toggle_cancels_it`
- `test_rubber_band_is_drawn_while_dragging` → `src/ui/region/selector.rs::rubber_band_is_drawn_while_dragging`
- `test_show_displays_the_frozen_frame_at_full_opacity` → `src/ui/region/selector.rs::backdrop_and_dimmed_frame_are_placed_in_order`（Python 該測試沒有任何斷言；凍結畫面與暗化底圖的位置由此守住）
- `test_dim_image_is_placed_at_the_games_client_origin` → `src/ui/region/selector.rs::backdrop_and_dimmed_frame_are_placed_in_order`, `src/ui/region/selector.rs::dimmed_frame_is_placed_in_points_on_scaled_displays`
- `test_spotlight_shows_the_dragged_area_and_hides_on_the_next_press` → `src/ui/region/selector.rs::spotlight_shows_the_dragged_area_and_hides_on_the_next_press`
- `test_backdrop_is_placed_below_the_game_frame_when_given` → `src/ui/region/selector.rs::backdrop_and_dimmed_frame_are_placed_in_order`
- `test_hint_shows_in_its_own_opaque_window` → `src/ui/region/selector.rs::hint_text_is_shown`（Python 為避免提示畫在明暗變化的底圖上而另開一層不透明視窗；Rust 在同一個 viewport 內畫在最上層，沒有獨立視窗可斷言）
- `test_hint_window_closes_with_the_layer` → 不移植：Tk 專屬：提示是獨立的 Toplevel，要手動跟著選取層關閉；Rust 的提示每幀畫在選取畫面內，選取畫面一關就跟著沒了
- `test_hint_window_closes_after_a_completed_drag` → 不移植：Tk 專屬：同上，獨立 Toplevel 的生命週期；Rust 的提示不是獨立視窗
- `test_show_while_open_leaves_exactly_one_hint_window` → 不移植：Tk 專屬：同上，獨立 Toplevel 的數量；Rust 的提示不是獨立視窗

## tests/ui/settings/test_fields.py

- `test_provider_labels_are_translated` → `tests/config_services.rs::provider_labels_are_translated`
- `test_common_languages_include_english` → `tests/settings_window.rs::the_target_language_offers_the_common_languages`
- `test_ui_language_field_round_trips_language_code` → `tests/settings_language.rs::save_keeps_the_previewed_language`, `tests/settings_window.rs::saving_an_unchanged_window_sends_its_own_keys_with_the_ui_language`
- `test_validate_requires_model` → `tests/config_golden.rs::validate_matches_python`, `tests/config_services.rs::validate_service_reports_every_missing_required_field`
- `test_validate_requires_key_for_official_providers` → `tests/config_golden.rs::validate_matches_python`, `tests/config_services.rs::validate_service_reports_every_missing_required_field`
- `test_validate_requires_base_url_for_custom_only` → `tests/config_golden.rs::validate_matches_python`, `tests/config_services.rs::validate_service_reports_every_missing_required_field`
- `test_friendly_error_guesses_from_status_when_api_gave_no_message` → `tests/service_form.rs::friendly_error_guesses_from_status_when_api_gave_no_message`
- `test_friendly_error_prefers_the_api_message_over_status_guess` → `tests/service_form.rs::friendly_error_prefers_the_api_message_over_status_guess`
- `test_friendly_error_shows_connection_failure_reason` → `tests/service_form.rs::friendly_error_shows_connection_failure_reason`
- `test_filter_models_is_case_insensitive_substring` → `tests/settings_widgets.rs::filter_models_is_case_insensitive_substring`
- `test_filter_models_blank_query_returns_all` → `tests/settings_widgets.rs::filter_models_blank_query_returns_all`
- `test_validate_endpoint_fields_ignores_model` → `tests/config_services.rs::validate_endpoint_fields_ignores_the_model`
- `test_model_field_shows_fetched_models` → `tests/service_form.rs::model_field_shows_fetched_models`
- `test_model_field_unsupported_endpoint_hints_manual_input` → `tests/service_form.rs::model_field_unsupported_endpoint_hints_manual_input`
- `test_model_field_error_uses_friendly_message` → `tests/service_form.rs::model_field_error_uses_friendly_message`
- `test_refresh_result_reports_failures_for_providers_without_base_url` → `tests/service_form.rs::refresh_result_reports_failures_for_providers_without_base_url`（新補；Python 的回歸是寫 log 時對沒有 base_url 的服務商 KeyError，Rust 沒有該路徑，改守同一個使用者可見結果：錯誤顯示出來、按鈕不卡在載入中）
- `test_model_field_error_makes_urls_clickable` → `tests/service_form.rs::model_field_error_makes_urls_clickable`
- `test_model_field_typing_filters_fetched_options` → `tests/service_form.rs::model_field_typing_filters_fetched_options`
- `test_model_field_hint_aligns_with_input` → `tests/service_form.rs::model_field_hint_aligns_with_input`
- `test_model_field_posted_dropdown_keeps_no_grab` → 不移植：Tk 專屬：ttk Combobox 原生清單會 grab 滑鼠，要拆掉才能邊開邊改關鍵字；Rust 清單是 egui 彈窗，沒有 grab，開著時繼續打字照樣篩選（`tests/service_form.rs::model_field_typing_filters_fetched_options`）
- `test_model_field_unpost_closes_dropdown` → `tests/settings_widgets.rs::editable_combo_escape_closes_and_keeps_the_typed_text`
- `test_model_field_dropdown_closes_on_outside_click` → `tests/service_form.rs::model_dropdown_closes_on_outside_click_and_when_nothing_matches`
- `test_model_field_status_says_custom_name_is_allowed` → `tests/service_form.rs::model_field_shows_fetched_models`
- `test_model_field_unpost_after_destroy_is_safe` → 不移植：Tk 專屬：元件被銷毀後延遲排程的收合仍會觸發 `TclError`；egui 沒有會被銷毀的 widget 物件，彈窗狀態每幀重建
- `test_posted_dropdown_reloads_while_typing` → `tests/service_form.rs::model_field_typing_filters_fetched_options`, `tests/settings_widgets.rs::editable_combo_filters_while_typing_and_picks_with_enter`
- `test_dropdown_closes_when_nothing_matches` → `tests/service_form.rs::model_dropdown_closes_on_outside_click_and_when_nothing_matches`, `tests/settings_widgets.rs::editable_combo_stays_closed_when_nothing_matches`
- `test_link_label_opens_the_url_on_click` → `tests/settings_window.rs::about_tab_shows_version_and_links`（點作者連結開出作者網址）
- `test_parse_link_markup_plain_text_has_no_links` → `tests/richtext_golden.rs::link_markup_matches_python`
- `test_parse_link_markup_splits_links_from_surrounding_text` → `tests/richtext_golden.rs::link_markup_matches_python`
- `test_parse_link_markup_rejects_non_http_urls` → `tests/richtext_golden.rs::link_markup_matches_python`
- `test_parse_link_markup_leaves_broken_syntax_as_text` → `tests/richtext_golden.rs::link_markup_matches_python`
- `test_linked_text_makes_only_the_link_segment_clickable` → `tests/settings_window.rs::about_tab_shows_the_translators`, `tests/settings_language.rs::translators_follow_the_previewed_language`（掛名文字中只有連結段是 `Role::Link`）

## tests/ui/settings/test_form.py

- `test_start_disables_the_button_and_delivers_the_result_on_the_main_thread` → `tests/settings_widgets.rs::round_discards_invalidated_and_stale_results`, `tests/service_form.rs::testing_disables_the_button_until_the_result_arrives`（Rust 的結果以 `UiEvent` 送回 UI 執行緒，沒有 Python 的背景執行緒與佇列）
- `test_an_exception_in_work_is_handed_to_on_done` → 不移植：Python 才有的機制：背景執行緒丟出的例外要接住交給 `on_done`。Rust 的工作回傳 `Result`，錯誤走 `TestDone`／`ModelsDone`／`UpdateChecked` 事件（`tests/service_form.rs::a_failed_test_shows_the_friendly_error`），按鈕照樣還原。已知缺口：任務內若 panic 不會轉成事件，按鈕會停在進行中，僅 bug 才會發生，未處理
- `test_a_result_that_arrives_after_invalidate_is_discarded_but_the_button_recovers` → `tests/settings_widgets.rs::round_discards_invalidated_and_stale_results`, `tests/service_form.rs::a_result_after_editing_is_discarded_and_the_button_recovers`
- `test_a_slow_round_cannot_land_in_a_later_round` → `tests/settings_widgets.rs::round_discards_invalidated_and_stale_results`
- `test_collapsible_starts_collapsed` → `tests/settings_widgets.rs::collapsible_starts_as_asked_and_toggles_on_click`
- `test_collapsible_can_start_expanded` → `tests/settings_widgets.rs::collapsible_starts_as_asked_and_toggles_on_click`
- `test_collapsible_toggles` → `tests/settings_widgets.rs::collapsible_starts_as_asked_and_toggles_on_click`
- `test_collapsible_header_is_keyboard_activatable` → `tests/settings_widgets.rs::collapsible_header_is_keyboard_activatable`
- `test_keyboard_activatable_shows_and_clears_the_focus_border` → 不移植：Tk 專屬：ttk 沒有 highlightthickness，聚焦樣式靠把邊框畫粗再還原；Rust 的 `collapsible` 在 `has_focus()` 時自己畫強調色框，鍵盤走得到、切得動由 `tests/settings_widgets.rs::collapsible_header_is_keyboard_activatable` 守住

## tests/ui/settings/test_provider_picker.py

- `test_one_card_per_provider` → `tests/service_list.rs::one_titled_card_per_provider`
- `test_each_card_titles_its_provider` → `tests/service_list.rs::one_titled_card_per_provider`
- `test_each_card_describes_its_provider` → `tests/service_list.rs::each_card_describes_its_provider`
- `test_clicking_a_card_reports_its_provider` → `tests/service_list.rs::clicking_anywhere_inside_a_card_picks_its_provider`
- `test_clicking_the_text_inside_a_card_counts_too` → `tests/service_list.rs::clicking_anywhere_inside_a_card_picks_its_provider`
- `test_cancel_leaves_the_result_none` → `tests/service_list.rs::add_service_opens_nothing_when_the_pick_is_cancelled`, `tests/service_list.rs::escape_cancels_the_provider_pick`
- `test_picker_geometry_carries_a_position` → 不移植：Tk 專屬：測試用的視窗停放 fixture 只改寫帶 `+x+y` 的 geometry 字串；Rust 的選擇框是設定視窗內的 egui 對話層，沒有獨立視窗位置
- `test_picker_grabs_input_and_releases_it_on_close` → `tests/service_list.rs::an_open_dialog_blocks_the_pane_behind_it`, `tests/service_list.rs::picking_from_the_form_switches_its_provider_and_keeps_it_modal`
- `test_picker_hands_the_grab_back_to_a_modal_parent` → `tests/service_list.rs::picking_from_the_form_switches_its_provider_and_keeps_it_modal`
- `test_escape_is_wired_to_cancel` → `tests/service_list.rs::escape_cancels_the_provider_pick`
- `test_the_cancel_button_closes_the_picker` → `tests/service_list.rs::the_cancel_button_closes_the_picker`（新補；第 4 段經使用者核可的重新設計：選擇框以標題列的關閉鈕取消，沒有獨立的〔取消〕鈕）
- `test_the_window_title_is_not_repeated_inside_the_window` → `tests/service_list.rs::the_picker_title_appears_once`
- `test_the_cards_keep_a_margin_below_the_window_top` → `tests/service_list.rs::the_picker_cards_keep_a_margin_below_the_header`
- `test_cards_are_reachable_and_activatable_by_keyboard` → `tests/service_list.rs::cards_are_reachable_and_activatable_by_keyboard`
- `test_each_card_shows_its_providers_icon` → `tests/service_list.rs::each_card_shows_its_providers_icon`
- `test_clicking_anywhere_inside_a_card_picks_its_provider` → `tests/service_list.rs::clicking_anywhere_inside_a_card_picks_its_provider`
- `test_clicking_the_icon_picks_the_provider` → `tests/service_list.rs::clicking_anywhere_inside_a_card_picks_its_provider`（該測試的點擊位置含圖示）
- `test_the_picker_takes_the_foreground_when_shown` → 不移植：第 4 段經使用者核可的重新設計：選擇框是設定視窗或精靈視窗內的 egui 對話層，不是另一個作業系統視窗，沒有「新視窗不被啟用」的問題；宿主視窗本身搶前景由 `tests/settings_host.rs::the_settings_window_takes_the_foreground_when_shown` 與 `the_wizard_takes_the_foreground_when_shown` 守住

## tests/ui/settings/test_responsive.py

- `test_wrap_width_subtracts_reserved_space` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_wrap_width_never_goes_below_minimum` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_apply_wrap_sets_wraplength_and_reports_change` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_apply_wrap_reports_no_change_when_width_is_same` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_bind_wrap_follows_container_width` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_bind_wrap_reserves_space_taken_by_earlier_widgets_in_the_row` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_bind_wrap_ignores_unmapped_container` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_apply_wrap_handles_label_without_wraplength` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住
- `test_bind_wrap_settles_on_the_same_width_regardless_of_history` → 不移植：Tk 專屬：`wraplength` 要自己依容器寬度算並寫回（避免 Configure 回圈、過渡寬度）；egui 的 Label 每幀以實際可用寬度排版換行，沒有可斷言的換行寬度。窄列不溢出的使用者可見結果由 `tests/settings_widgets.rs::dropdowns_give_way_to_the_label_in_a_narrow_row` 守住

## tests/ui/settings/test_scrollable.py

- `test_body_is_where_content_goes` → 不移植：Tk 專屬：自製 `ScrollableFrame` 的 body 容器；Rust 的捲動區是 `egui::ScrollArea`，滾輪與內容寬度由 egui 處理
- `test_scrollbar_packs_before_the_canvas` → 不移植：Tk 專屬：pack 順序（捲軸先於 canvas）；Rust 的捲動區是 `egui::ScrollArea`，滾輪與內容寬度由 egui 處理
- `test_wheel_does_nothing_when_content_fits` → 不移植：Tk 專屬：Canvas 的滾輪處理（內容未溢出時不捲）；Rust 的捲動區是 `egui::ScrollArea`，滾輪與內容寬度由 egui 處理
- `test_wheel_scrolls_when_content_overflows` → 不移植：Tk 專屬：Canvas 的滾輪處理（內容溢出時捲動）；Rust 的捲動區是 `egui::ScrollArea`，滾輪與內容寬度由 egui 處理
- `test_inner_width_follows_canvas` → 不移植：Tk 專屬：內層 frame 要手動跟著 canvas 寬度；Rust 的捲動區是 `egui::ScrollArea`，滾輪與內容寬度由 egui 處理
- `test_settings_button_row_is_packed_before_the_notebook` → `tests/settings_window.rs::the_footer_stays_visible_in_a_small_window`（pack 順序是 Tk 機制，改守其使用者可見結果：視窗變矮時按鈕列仍在視窗內）
- `test_wizard_nav_is_packed_before_the_body` → `tests/wizard.rs::wizard_nav_is_packed_before_the_body`（新補；改守使用者可見結果：內容比視窗高時，上一步／下一步仍在視窗內）
- `test_overlay_error_banner_is_packed_before_the_scroll_area` → `src/ui/app.rs::error_banner_not_squeezed_by_list`（改守使用者可見結果：橫幅不被訊息列表擠掉）
- `test_overlay_title_is_packed_after_the_bar_controls` → `src/ui/overlay/frame.rs::title_buttons_never_clipped_by_long_name`（改守使用者可見結果：標題再長也不會把 ✕／⚙／─ 擠出視窗）
- `test_secret_entry_reveal_button_is_packed_before_the_entry` → `tests/settings_widgets.rs::the_secret_eye_stays_inside_the_field_and_flips_its_icon`（改守使用者可見結果：顯示金鑰的眼睛按鈕留在欄位內）

## tests/ui/settings/test_service_form.py

- `test_new_form_starts_from_the_providers_defaults` → `tests/service_form.rs::new_form_starts_from_the_providers_defaults`
- `test_editing_keeps_the_id` → `tests/service_form.rs::editing_keeps_the_id`
- `test_values_only_carry_the_fields_that_provider_has` → `tests/service_form.rs::values_only_carry_the_fields_that_provider_has`
- `test_api_values_drop_the_bookkeeping_fields` → `tests/service_form.rs::api_values_drop_the_bookkeeping_fields`
- `test_switching_provider_rewrites_an_untouched_name` → `tests/service_form.rs::switching_provider_rewrites_an_untouched_name`
- `test_switching_provider_keeps_a_name_the_user_typed` → `tests/service_form.rs::switching_provider_keeps_a_name_the_user_typed`
- `test_switching_provider_rewrites_a_suffixed_untouched_name` → `tests/service_form.rs::switching_provider_rewrites_a_suffixed_untouched_name`
- `test_a_blank_name_falls_back_to_the_provider_short_name` → `tests/service_form.rs::a_blank_name_falls_back_to_the_provider_short_name`
- `test_switching_provider_dedupes_the_new_name_in_the_field` → `tests/service_form.rs::switching_provider_dedupes_the_new_name_in_the_field`
- `test_switching_provider_to_a_free_short_name_has_no_suffix` → `tests/service_form.rs::switching_provider_to_a_free_short_name_has_no_suffix`
- `test_editing_a_service_switching_away_and_back_keeps_its_bare_name` → `tests/service_form.rs::editing_a_service_switching_away_and_back_keeps_its_bare_name`
- `test_switching_provider_keeps_a_typed_name_even_with_a_collision` → `tests/service_form.rs::switching_provider_keeps_a_typed_name_even_with_a_collision`
- `test_blank_name_falls_back_to_the_deduped_short_name` → `tests/service_form.rs::blank_name_falls_back_to_the_deduped_short_name`
- `test_switching_provider_does_not_carry_values_across` → `tests/service_form.rs::switching_provider_does_not_carry_values_across`
- `test_claude_keeps_its_effort_choice` → `tests/service_form.rs::claude_keeps_its_effort_choice`
- `test_switching_provider_takes_the_new_providers_effort_default` → `tests/service_form.rs::switching_provider_takes_the_new_providers_effort_default`
- `test_switching_provider_clears_the_test_result` → `tests/service_form.rs::switching_provider_clears_the_test_result`
- `test_claude_shows_thinking_depth_instead_of_a_toggle` → `tests/service_form.rs::claude_shows_thinking_depth_instead_of_a_toggle`
- `test_the_other_providers_keep_the_thinking_toggle` → `tests/service_form.rs::the_other_providers_keep_the_thinking_toggle`
- `test_the_effort_dropdown_shows_the_saved_choice` → `tests/service_form.rs::the_effort_dropdown_shows_the_saved_choice`
- `test_choosing_an_effort_stores_its_code` → `tests/service_form.rs::choosing_an_effort_stores_its_code`
- `test_switching_provider_clears_the_fetched_model_list` → `tests/service_form.rs::switching_provider_clears_the_fetched_model_list`
- `test_a_successful_test_shows_the_sample_translation` → `tests/service_form.rs::a_successful_test_shows_time_source_and_translation`（第 4 段經使用者核可的重新設計：成功訊息改為顯示耗時秒數，並分列原文與譯文）
- `test_a_failed_test_shows_the_friendly_error` → `tests/service_form.rs::a_failed_test_shows_the_friendly_error`
- `test_the_test_target_language_defaults_to_the_ui_language` → `tests/wizard.rs::the_test_translates_into_the_interface_language`, `tests/service_form.rs::test_connection_sends_the_form_values_and_target`
- `test_the_provider_row_shows_the_current_provider` → `tests/service_form.rs::the_provider_row_shows_the_current_provider`
- `test_the_change_button_switches_the_provider` → `tests/service_form.rs::the_change_button_asks_the_owner_to_pick_a_provider`, `tests/service_list.rs::picking_from_the_form_switches_its_provider_and_keeps_it_modal`
- `test_cancelling_the_provider_pick_changes_nothing` → `tests/service_list.rs::cancelling_the_provider_pick_changes_nothing`（新補）
- `test_editing_a_field_notifies_the_owner` → `tests/service_form.rs::editing_a_field_notifies_the_owner`
- `test_rebuilding_the_fields_notifies_once` → `tests/service_form.rs::switching_provider_notifies_once`
- `test_test_connection_also_generates_examples` → `tests/translator.rs::test_translate_generates_and_stores_examples`（`test_translate` 帶 `ExampleStore` 時會生成並存範例；`RealEnv::test_connection` 傳入該 store 的那一行接線需要真實網路與檔案，沒有自動測試）

## tests/ui/settings/test_service_list.py

- `test_values_round_trip_unchanged` → `tests/service_list.rs::values_round_trip_unchanged`
- `test_cards_are_listed_in_order_with_the_default_badged` → `tests/service_list.rs::cards_are_listed_in_order_with_the_default_badged`
- `test_a_card_with_no_role_shows_its_name_alone` → `tests/service_list.rs::a_card_with_no_role_shows_its_name_alone`
- `test_the_default_badge_comes_before_the_slot_badges` → `tests/service_list.rs::the_default_badge_comes_before_the_slot_badges`
- `test_several_slots_on_one_service_are_badged_in_slot_order` → `tests/service_list.rs::several_slots_on_one_service_are_badged_in_slot_order`
- `test_deleting_the_default_hands_it_to_the_first_remaining` → `tests/service_list.rs::deleting_the_default_hands_it_to_the_first_remaining`
- `test_cancelling_delete_changes_nothing` → `tests/service_list.rs::deleting_needs_confirmation_and_cancel_changes_nothing`
- `test_deleting_an_assigned_service_frees_that_slot` → `tests/service_list.rs::deleting_an_assigned_service_frees_that_slot`
- `test_the_last_service_cannot_be_deleted` → `tests/service_list.rs::the_last_service_cannot_be_deleted`
- `test_adding_to_an_empty_list_makes_it_the_default` → `tests/service_list.rs::adding_to_an_empty_list_makes_it_the_default`
- `test_a_saved_name_never_collides` → `tests/service_list.rs::a_saved_name_never_collides`
- `test_slots_collapsed_when_everything_follows_the_default` → `tests/service_list.rs::slots_collapsed_when_everything_follows_the_default`
- `test_slots_expanded_when_one_is_assigned` → `tests/service_list.rs::slots_expanded_when_one_is_assigned`
- `test_slot_options_start_with_follow_the_default` → `tests/service_list.rs::slot_options_start_with_follow_the_default`
- `test_dialog_ok_keeps_result_none_when_invalid` → `tests/service_list.rs::dialog_ok_keeps_the_dialog_open_when_invalid`
- `test_dialog_ok_saves_full_service_when_valid` → `tests/service_list.rs::dialog_ok_saves_the_full_service_when_valid`
- `test_dialog_cancel_leaves_result_none` → `tests/service_list.rs::dialog_cancel_changes_nothing`
- `test_dialog_geometry_carries_a_position` → 不移植：Tk 專屬：測試用的視窗停放 fixture 只改寫帶 `+x+y` 的 geometry 字串；Rust 的服務對話框是設定視窗內的 egui 對話層，沒有獨立視窗位置
- `test_dialog_grabs_input_and_releases_it_on_close` → `tests/service_list.rs::an_open_dialog_blocks_the_pane_behind_it`, `tests/service_list.rs::clicking_the_scrim_does_not_close_the_dialog`
- `test_add_service_opens_nothing_when_the_pick_is_cancelled` → `tests/service_list.rs::add_service_opens_nothing_when_the_pick_is_cancelled`
- `test_add_service_drafts_the_picked_provider` → `tests/service_list.rs::add_service_drafts_the_picked_provider`
- `test_assigning_a_slot_rebadges_the_cards` → `tests/service_list.rs::assigning_a_slot_rebadges_the_cards`
- `test_a_long_name_does_not_squeeze_the_card_buttons` → `tests/service_list.rs::a_long_name_does_not_squeeze_the_card_buttons`
- `test_dialog_escape_is_wired_to_cancel` → `tests/service_list.rs::dialog_escape_cancels`
- `test_the_last_service_says_why_delete_is_greyed_out` → `tests/service_list.rs::the_last_service_says_why_delete_is_greyed_out`
- `test_the_keep_one_hint_goes_away_once_there_are_two` → `tests/service_list.rs::the_keep_one_hint_goes_away_once_there_are_two`
- `test_deleting_an_assigned_service_says_its_slots_fall_back` → `tests/service_list.rs::deleting_an_assigned_service_says_its_slots_fall_back`
- `test_deleting_an_unassigned_service_keeps_the_plain_question` → `tests/service_list.rs::deleting_an_unassigned_service_keeps_the_plain_question`
- `test_every_provider_ships_an_icon` → `tests/service_list.rs::every_provider_ships_an_icon`
- `test_each_card_shows_its_providers_icon` → `tests/service_list.rs::each_card_shows_its_providers_icon`
- `test_the_card_icon_is_centred_against_the_whole_card` → `tests/service_list.rs::each_card_shows_its_providers_icon`
- `test_a_card_without_its_icon_file_still_renders` → `tests/service_list.rs::a_card_without_its_icon_still_renders`
- `test_the_service_dialog_takes_the_foreground_when_shown` → 不移植：第 4 段經使用者核可的重新設計：服務對話框是設定視窗內的 egui 對話層，不是另一個作業系統視窗，沒有「新視窗不被啟用」的問題；宿主視窗搶前景由 `tests/settings_host.rs::the_settings_window_takes_the_foreground_when_shown` 守住

## tests/ui/settings/test_window.py

- `test_clamp_advanced_limits_ranges` → `tests/config_services.rs::parse_advanced_clamps_valid_numeric_input`, `tests/config_golden.rs::load_matches_python`
- `test_clamp_advanced_passes_valid_values` → `tests/config_services.rs::parse_advanced_passes_valid_values_through`
- `test_overlay_set_limits_trims_messages` → `src/ui/app.rs::limits_trim_messages_and_fading_wakes_at_the_next_expiry`, `src/ui/overlay/model.rs::max_messages_drops_oldest`
- `test_overlay_set_alpha_applies_to_backdrop_and_bubble` → `src/ui/overlay/frame.rs::set_alpha_applies_immediately`, `src/ui/overlay/bubble.rs::bubble_alpha_floor`（泡泡比主視窗再透一成由 `bubble_alpha_floor` 守住；Rust 是單層視窗，沒有 backdrop 與文字層之分，「文字層維持不透明」無從斷言）
- `test_bubble_alpha_stops_at_the_floor` → `src/ui/overlay/bubble.rs::bubble_alpha_floor`
- `test_overlay_backdrop_follows_geometry` → 不移植：Tk 專屬：雙層視窗（底板 backdrop 與文字層）要各自同步位置與大小；Rust 的 overlay 只有一個視窗
- `test_parse_advanced_values_returns_error_on_non_numeric_input` → `tests/config_services.rs::parse_advanced_rejects_non_numbers_and_non_finite`, `tests/settings_window.rs::a_non_number_advanced_value_blocks_the_save`
- `test_parse_advanced_values_clamps_valid_numeric_input` → `tests/config_services.rs::parse_advanced_clamps_valid_numeric_input`
- `test_parse_advanced_values_clamps_parallel` → `tests/config_services.rs::parse_advanced_clamps_parallel`
- `test_parse_advanced_values_returns_error_key` → `tests/config_services.rs::parse_advanced_rejects_non_numbers_and_non_finite`
- `test_save_applies_ui_language` → `tests/settings_language.rs::save_keeps_the_previewed_language`, `tests/settings_host.rs::gear_opens_settings_and_save_applies_to_the_overlay`
- `test_save_stores_the_system_message_toggle` → `tests/settings_window.rs::save_stores_the_system_message_toggle`
- `test_reopening_settings_reflects_the_saved_toggle` → `tests/settings_window.rs::reopening_settings_reflects_the_saved_toggle`
- `test_the_services_tab_sits_between_basic_and_advanced` → `tests/settings_window.rs::the_services_tab_sits_between_basic_and_advanced`
- `test_save_writes_the_edited_service_list_into_cfg` → `tests/settings_window.rs::save_writes_the_edited_service_list_into_cfg`
- `test_save_refuses_an_incomplete_default_service` → `tests/settings_window.rs::save_refuses_an_incomplete_default_service`
- `test_both_language_fields_come_before_the_hotkeys` → `tests/settings_window.rs::both_language_fields_come_before_the_hotkeys`
- `test_service_summary_row_sits_between_target_language_and_hotkey_label` → `tests/settings_window.rs::service_summary_row_sits_between_target_language_and_hotkey_label`
- `test_manage_button_selects_the_services_tab` → `tests/settings_window.rs::manage_button_switches_to_the_services_page`
- `test_service_summary_shows_the_draft_default_service_name` → `tests/settings_window.rs::service_summary_shows_the_draft_default_service_name`
- `test_service_summary_refreshes_when_the_default_changes_and_tabs_switch` → `tests/settings_window.rs::service_summary_refreshes_when_the_default_changes`
- `test_changing_ui_language_previews_it_without_touching_config` → `tests/settings_language.rs::changing_ui_language_previews_it_without_touching_config`
- `test_language_preview_flushes_the_overlay_repaint_before_rebuilding` → 不移植：Tk 專屬：換語言要先 `root.update()` 讓 overlay 重繪，再重建設定視窗，否則舊語言多出的字會殘留約 0.3 秒；Rust 的視窗不重建，egui 每幀整個重畫，沒有殘影。overlay 隨語言重貼標籤由 `tests/overlay_relabel.rs::status_key_relabels_on_language_change` 守住
- `test_language_preview_keeps_unsaved_edits` → `tests/settings_language.rs::language_preview_keeps_unsaved_edits`, `tests/settings_language.rs::language_preview_keeps_the_draft_and_cancel_reverts`（`language_preview_keeps_unsaved_edits` 新補，涵蓋框選熱鍵與自動開輸入框開關）
- `test_language_preview_keeps_the_current_tab` → `tests/settings_language.rs::language_preview_keeps_the_draft_and_cancel_reverts`（換語言後仍在原頁；Rust 的介面語言下拉只在基本頁，視窗不重建，頁面狀態原樣保留）
- `test_cancel_restores_the_previewed_language` → `tests/settings_language.rs::language_preview_keeps_the_draft_and_cancel_reverts`, `tests/settings_language.rs::cancel_through_the_window_close_reverts_too`
- `test_save_keeps_the_previewed_language` → `tests/settings_language.rs::save_keeps_the_previewed_language`
- `test_about_tab_shows_version_and_links` → `tests/settings_window.rs::about_tab_shows_version_and_links`
- `test_about_tab_shows_the_log_folder` → `tests/settings_window.rs::about_tab_shows_the_log_folder`
- `test_manual_check_reports_up_to_date` → `tests/settings_window.rs::manual_check_reports_up_to_date`
- `test_manual_check_hands_a_new_version_to_on_update_found` → `tests/settings_window.rs::manual_check_hands_a_new_version_to_update_found`
- `test_manual_check_keeps_on_update_found_quiet_when_up_to_date` → `tests/settings_window.rs::manual_check_keeps_update_found_quiet_when_up_to_date`
- `test_manual_check_reports_a_new_version` → `tests/settings_window.rs::manual_check_reports_a_new_version`
- `test_manual_check_reports_failure` → `tests/settings_window.rs::manual_check_reports_failure`
- `test_check_button_runs_the_real_thread_and_poll_path` → `tests/settings_window.rs::checking_disables_the_button_until_the_result`, `tests/settings_host.rs::settings_events_reach_the_open_window`（結果經 `UiEvent` 回 UI 執行緒，沒有 Python 的執行緒與 `poll_queue`）
- `test_update_result_is_only_clickable_over_its_text` → `tests/settings_window.rs::update_result_is_only_clickable_over_its_text`
- `test_about_tab_hides_the_cache_row_without_a_cache` → `tests/settings_window.rs::about_tab_hides_the_cache_row_without_a_cache`
- `test_clear_cache_button_clears_and_reports_the_count` → `tests/settings_window.rs::clear_cache_button_clears_and_reports_the_count`
- `test_clear_cache_reports_zero_when_the_cache_was_already_empty` → `tests/settings_window.rs::clear_cache_reports_zero_when_the_cache_was_already_empty`
- `test_translators_row_follows_the_ui_language_field` → `tests/settings_window.rs::translators_row_follows_the_ui_language_field`
- `test_help_translate_link_opens_crowdin` → `tests/settings_window.rs::help_translate_link_opens_crowdin`
- `test_about_tab_shows_the_translators` → `tests/settings_window.rs::about_tab_shows_the_translators`
- `test_about_tab_links_to_crowdin` → `tests/settings_window.rs::about_tab_shows_version_and_links`, `tests/settings_window.rs::about_tab_shows_the_translators`（Crowdin 連結在關於頁、排在譯者之後、回報問題之前）
- `test_translators_rows_are_hidden_when_the_language_credits_nobody` → `tests/settings_language.rs::translators_rows_are_hidden_when_the_language_credits_nobody`
- `test_translators_follow_the_previewed_language` → `tests/settings_language.rs::translators_follow_the_previewed_language`
- `test_settings_window_does_not_stay_on_top` → `tests/settings_host.rs::settings_window_does_not_stay_on_top`
- `test_releasing_topmost_survives_a_window_closed_in_the_meantime` → `tests/settings_host.rs::releasing_topmost_survives_a_window_closed_in_the_meantime`
- `test_paste_hotkey_defaults_on_and_saves_from_the_settings_toggle` → `tests/settings_window.rs::paste_hotkey_defaults_on_and_saves_from_the_settings_toggle`
- `test_save_stores_the_region_hotkey` → `tests/settings_window.rs::save_stores_the_region_hotkey`
- `test_save_rejects_identical_hotkeys` → `tests/settings_window.rs::save_rejects_identical_hotkeys`
- `test_language_preview_keeps_a_half_added_service` → `tests/settings_language.rs::language_preview_keeps_a_half_added_service`
- `test_save_refuses_a_service_a_slot_points_at` → `tests/settings_window.rs::save_refuses_a_service_a_slot_points_at`
- `test_save_lists_a_shared_services_problem_only_once` → `tests/settings_window.rs::save_lists_a_shared_services_problem_only_once`
- `test_the_basic_tab_labels_the_service_row_with_the_default_service` → `tests/settings_window.rs::the_basic_tab_labels_the_service_row_with_the_default_service`
- `test_the_settings_window_takes_the_foreground_when_shown` → `tests/settings_host.rs::the_settings_window_takes_the_foreground_when_shown`

## tests/ui/settings/test_wizard.py

- `test_step_api_requires_valid_form` → `tests/wizard.rs::step_service_requires_valid_form`
- `test_step_api_requires_passed_or_skipped_test` → `tests/wizard.rs::step_service_requires_passed_or_skipped_test`
- `test_step_prefs_always_advances` → `tests/wizard.rs::step_prefs_always_advances`
- `test_can_advance_allows_language_step` → `tests/wizard.rs::can_advance_allows_language_step`
- `test_language_change_marks_restart_and_applies_language` → `tests/wizard_language.rs::language_change_relabels_the_same_wizard`（Rust 的精靈就地換語言、不重建視窗，沒有 `restart` 旗標；換語言立即套用，完成時寫入的設定帶新語言）
- `test_language_change_updates_untouched_target_language` → `tests/wizard_language.rs::language_change_updates_untouched_target_language`
- `test_language_change_keeps_customised_target_language` → `tests/wizard_language.rs::language_change_keeps_customised_target_language`
- `test_language_change_keeps_the_half_filled_service` → `tests/wizard_language.rs::language_change_keeps_the_half_filled_service`
- `test_language_change_retitles_an_untouched_service` → `tests/wizard_language.rs::language_change_retitles_an_untouched_service`
- `test_language_change_retitles_a_suffixed_untouched_service` → `tests/wizard_language.rs::language_change_retitles_a_suffixed_untouched_service`
- `test_language_change_only_retitles_the_service_being_edited` → `tests/wizard_language.rs::language_change_only_retitles_the_service_being_edited`
- `test_language_change_keeps_a_service_name_the_user_typed` → `tests/wizard_language.rs::language_change_keeps_a_service_name_the_user_typed`
- `test_language_change_from_en_bootstrap_follows_to_zh_cn` → `tests/wizard_language.rs::language_change_from_en_bootstrap_follows_to_zh_cn`
- `test_bootstrap_language_first_run_no_config_file` → `tests/config_golden.rs::bootstrap_matches_python`
- `test_bootstrap_language_upgrading_user_keeps_target_language` → `tests/config_golden.rs::bootstrap_matches_python`
- `test_bootstrap_language_returning_user_uses_saved_language` → `tests/config_golden.rs::bootstrap_matches_python`
- `test_language_step_shows_the_translators` → `tests/wizard.rs::language_step_shows_the_translators`
- `test_language_step_without_translators_shows_no_row` → `tests/wizard_language.rs::translators_follow_the_language`
- `test_language_step_links_to_crowdin_even_without_translators` → `tests/wizard_language.rs::translators_follow_the_language`, `tests/wizard.rs::language_step_links_to_crowdin`
- `test_finishing_the_wizard_keeps_the_other_services` → `tests/wizard.rs::finishing_the_wizard_keeps_the_other_services`
- `test_finish_dedupes_a_name_typed_to_match_another_service` → `tests/wizard.rs::finish_dedupes_a_name_typed_to_match_another_service`
- `test_finish_does_not_collide_a_service_with_itself` → `tests/wizard_language.rs::finish_does_not_collide_a_service_with_itself`
- `test_finish_stores_the_region_hotkey` → `tests/wizard.rs::finish_stores_the_preferences`
- `test_finish_rejects_identical_hotkeys` → `tests/wizard.rs::finish_rejects_identical_hotkeys`
- `test_language_change_does_not_let_the_rename_collide` → `tests/wizard_language.rs::language_change_does_not_let_the_rename_collide`
- `test_switching_provider_in_the_wizard_dedupes_against_cfg_services` → `tests/wizard.rs::switching_provider_in_the_wizard_dedupes_against_cfg_services`
- `test_step_api_asks_for_a_provider_before_the_form` → `tests/wizard.rs::step_service_asks_for_a_provider_before_the_form`
- `test_picking_a_provider_replaces_the_cards_with_the_form` → `tests/wizard.rs::picking_a_provider_replaces_the_cards_with_the_form`
- `test_an_existing_service_skips_the_cards` → `tests/wizard.rs::an_existing_service_skips_the_cards`
- `test_language_change_while_picking_rebuilds_into_the_cards` → `tests/wizard_language.rs::language_change_while_picking_stays_on_the_cards`
- `test_language_change_after_picking_rebuilds_into_the_form` → `tests/wizard_language.rs::language_change_after_picking_keeps_the_form`
- `test_the_form_instructions_wait_until_the_form_exists` → `tests/wizard.rs::the_form_instructions_wait_until_the_form_exists`
- `test_the_form_step_explains_what_to_do_with_the_form` → `tests/wizard.rs::the_form_step_explains_what_to_do_with_the_form`
- `test_clearing_a_field_after_skipping_the_test_blocks_the_next_button` → `tests/wizard.rs::clearing_a_field_after_skipping_the_test_blocks_the_next_button`
- `test_editing_a_field_cancels_an_earlier_skip` → `tests/wizard.rs::editing_a_field_cancels_an_earlier_skip`
- `test_completing_the_fields_after_an_early_skip_recomputes_the_nav` → `tests/wizard.rs::completing_the_fields_after_an_early_skip_recomputes_the_nav`
- `test_the_skip_test_link_is_keyboard_activatable` → `tests/wizard.rs::the_skip_test_link_is_keyboard_activatable`
- `test_the_wizard_takes_the_foreground_when_shown` → `tests/settings_host.rs::the_wizard_takes_the_foreground_when_shown`
