{
  pkgs,
  flower,
  inputs ? [ ],
}:
flow:
let
  inherit (pkgs.lib) concatMapStrings escapeShellArg;

  deps = builtins.fromTOML (
    builtins.readFile (
      pkgs.runCommandLocal "flow-deps.toml" { } ''
        ${flower}/bin/flower deps ${flow} --out $out
      ''
    )
  );

  nodes = builtins.mapAttrs (
    name:
    { deps }:
    pkgs.runCommand name { inherit inputs; } ''
      mkdir run && cd run
      ${concatMapStrings (dep: "ln -s ${nodes.${dep}} ${escapeShellArg dep}\n") deps}
      ${flower}/bin/flower run --flow ${flow} --node ${escapeShellArg name} --no-history --no-deps --dir .
      mv ${escapeShellArg name} $out
    ''
  ) deps;
in
nodes
