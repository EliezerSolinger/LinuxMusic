//! Spectrum Analyzer: the contract (the audio passes through unchanged; the worker's analysis is the window's)

lm_plugin::contract_tests!(lm_plugins_effects::spectrum_analyzer::DESCRIPTOR, "spectrumanalyzer");
