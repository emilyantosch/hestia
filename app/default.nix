{
  lib,
  rustPlatform,
  pkg-config,
  rustfmt,
  cacert,
  openssl,
  nix-update-script,
}:

rustPlatform.buildRustPackage rec {
  pname = "hestia";
  version = "0.1.0";

  src = ./.; 

  cargoHash = "";

  nativeBuildInputs = [
    pkg-config
    cacert
  ];

  buildInputs = [ openssl ];

  OPENSSL_NO_VENDOR = 1;

  nativeCheckInputs = [ rustfmt ];

  checkFlags = [
    # requires network access
    "--skip=serve::proxy::test"
  ];

  passthru = {
    updateScript = nix-update-script { };
  };

  meta = with lib; {
    homepage = "...";
    description = "...";
    changelog = "...";
    license = with licenses; [
      mit
      asl20
    ];
    maintainers = with maintainers; [
      emilyantosch
    ];
    mainProgram = "hestia";
  };
}
