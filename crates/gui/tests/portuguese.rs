//! The bundled Brazilian Portuguese catalog, through the same Slint component
//! the app and the right-click task window use for their texts.
use hatchzip_gui::{MainWindow, Phase};
use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};

#[test]
fn bundled_portuguese() {
    slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
        mock_time: false,
        threading: true,
        renderer_name: Some("software".into()),
    })))
    .unwrap();
    let ui = MainWindow::new().unwrap();
    slint::select_bundled_translation("pt").unwrap();
    assert_eq!(
        ui.invoke_task_heading(Phase::Done, true, false, "a.zip".into()),
        "Teste concluído"
    );
    assert_eq!(
        ui.invoke_task_heading(Phase::Working, false, false, "a.zip".into()),
        "Extraindo a.zip"
    );
    assert_eq!(ui.invoke_destination_title(), "Selecionar pasta de destino");
    // Brazilian Portuguese counts zero as singular.
    for (n, expected) in [(0, "0 entrada"), (1, "1 entrada"), (2, "2 entradas")] {
        assert_eq!(ui.invoke_entry_count(n.to_string().into(), n), expected);
    }
    assert_eq!(
        ui.invoke_task_test_result(1, "1".into()),
        "Teste aprovado: 1 entrada verificada."
    );
    assert_eq!(
        ui.invoke_task_test_result(3, "3".into()),
        "Teste aprovado: 3 entradas verificadas."
    );
}
