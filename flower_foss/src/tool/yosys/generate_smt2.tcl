# Converts a design and its properties into an SMT2 model of their state transitions.
# - top: the top module
# - flist: the flist of the design
# - props: a list of property files
# - smt2: the SMT2 file to write
proc generate_smt2 {top flist props smt2} {
  yosys echo on

  # parse SystemVerilog using the slang plugin
  yosys read_slang --top $top -f $flist {*}$props

  # create basic netlist
  yosys prep -top $top

  # ensure the hierarchy is compatible with SMT solvers
  yosys hierarchy -smtcheck

  # select strongly connected components
  yosys scc -select
  # simplify the selection
  yosys simplemap
  # clear the selection
  yosys select -clear

  # extract flip-flops from memory read ports
  yosys memory_nordff

  # convert async logic (likely resets) to synchronous logic
  yosys async2sync

  # process assume statements early
  yosys chformal -assume -early

  # remove unused wires and cells
  yosys opt_clean

  # formalise state transitions with:
  # - undefined initial states
  # - replace all clocked flip-flops with $ff cells (implicit global clock)
  yosys formalff -setundef -clk2ff -ff2anyinit -hierarchy

  # remove liveness, fairness and cover properties
  yosys chformal -live -fair -cover -remove

  # clean up unused wires and cells
  yosys opt_clean

  # check the circuit is still good, e.g. no combinational loops
  yosys check

  # handle undriven signals by making them non-deterministic
  yosys setundef -undriven -anyseq

  # optimize the circuit a little
  yosys opt -fast

  # ensure signals are still properly named for witness traces
  yosys rename -witness

  # final clean up of unused wires and cells
  yosys opt_clean

  # ensure the hierarchy is still compatible with SMT solvers
  yosys hierarchy -smtcheck

  # remove $print cells
  yosys delete {*/t:$print}

  # constrain wires to values they would have before a clock edge
  yosys formalff -assume

  # remove clock enable and synchronous reset from flip-flops, if they have them
  yosys dffunmap

  yosys write_smt2 -wires $smt2
}
