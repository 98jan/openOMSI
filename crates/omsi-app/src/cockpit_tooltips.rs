//! Localised cockpit labels, including names composed from mod trigger names.
use omsi_input::describe::ControlNames;

pub(crate) fn control(names: &ControlNames, event: &str, language: &str) -> String {
    if omsi_cfg::lang::language_code(language) == "PTB" {
        names.control_localized(event, |key| {
            crate::_rust_i18n_try_translate("pt", key).map(|text| text.into_owned())
        })
    } else {
        names.control(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cockpit_word_has_a_brazilian_portuguese_translation() {
        for key in omsi_input::describe::control_vocabulary() {
            assert!(
                crate::_rust_i18n_try_translate("pt", key).is_some(),
                "{key}"
            );
        }
    }

    #[test]
    fn official_labels_and_generated_mod_names_are_localized() {
        let names = ControlNames::from_table(
            "ENG",
            &[
                ("parking_brake_toggle", "Parking Brake On/Off"),
                ("bus_doorfront0", "Front Door, 1st Wing Open/Close"),
            ],
        );
        for language in ["PTB", "pt", "pt-BR", "por"] {
            for (event, expected) in [
                (
                    "cp_heizregler_temp",
                    "Controle de temperatura do aquecimento",
                ),
                (
                    "parking_brake_mouse",
                    "Freio de estacionamento: ligar/desligar",
                ),
                (
                    "bus_doorfront0_external",
                    "Porta dianteira, primeira folha: abrir/fechar (externo)",
                ),
                (
                    "cp_schalter_hub_dn_toggle",
                    "Interruptor de descida do elevador: ligar/desligar",
                ),
                (
                    "cp_spiegelheizung_toggle",
                    "Aquecimento dos espelhos: ligar/desligar",
                ),
                ("ALMEX_Funktion", "ALMEX função"),
                (
                    "Zahltisch_Wechsler_0_05",
                    "Mesa de cobrança trocador de moedas 0.05",
                ),
                (
                    "pax_door_request",
                    "Passageiro: pedir embarque / desembarque",
                ),
            ] {
                assert_eq!(
                    control(&names, event, language),
                    expected,
                    "{language}: {event}"
                );
            }
        }
        assert_eq!(
            control(&names, "cp_heizregler_temp", "ENG"),
            "Heating control temperature"
        );
        assert_eq!(
            control(&names, "parking_brake_mouse", "ENG"),
            "Parking Brake On/Off"
        );
    }
}
