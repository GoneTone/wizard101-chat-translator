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

- `test_stub_copies_sso_and_heap_strings_and_restores_registers` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`
- `test_stub_truncates_long_lines_and_keeps_original_length` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`
- `test_stub_wraps_past_the_slot_count` → `tests/hooks_golden.rs::addline_stub_bytes_match_python`
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
