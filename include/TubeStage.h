/*
 * TubeStage.h - a triode preamp stage: its circuit, solved per sample or approximated around its static curve
 *
 * This file is part of LinuxMusic, a fork of LMMS - https://lmms.io
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this program (see COPYING); if not, write to the
 * Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor,
 * Boston, MA 02110-1301 USA.
 *
 */

#ifndef LMMS_TUBE_STAGE_H
#define LMMS_TUBE_STAGE_H

#include <array>
#include <vector>

namespace lmms
{

/**
 * A triode, after Norman Koren ("Improved VT models for SPICE simulations", 1996) for its plate current, and
 * Kristjan Dempwolf and Udo Zölzer ("A physically-motivated triode model for circuit simulations", DAFx 2011) for its
 * grid current, with its interelectrode capacitances.
 */
struct Triode
{
	double mu, ex, kg1, kp, kvb;	//!< Koren's parameters
	double gridToPlate;				//!< F
	double gridToCathode;			//!< F

	//! The plate current for the grid and plate voltages to the cathode, and its derivatives
	struct Current { double value, byGrid, byPlate; };
	auto plateCurrent(double vgk, double vpk) const -> Current;
	//! The grid current for the grid voltage to the cathode, and its derivative
	auto gridCurrent(double vgk) const -> Current;
};

/**
 * A common cathode stage, as at the input of a tube preamp:
 *
 *   in -- Ci --+-- Rg --- grid      plate --- Rp --- B+ --- Rs --- supply
 *              |                    |                |
 *             Rgl       cathode     +-- Co --+       Cs
 *              |        |   |                |       |
 *             gnd      Rk   Ck              RL      gnd
 *                       |   |                |
 *                      gnd gnd              gnd (out)
 *
 * Ci and Rgl couple the input (Ci charges through the grid current: blocking), Rg is the grid stopper, which with the
 * Miller capacitance rolls off the treble, Ck bypasses the cathode resistor, Co and RL couple the output, and Rs and
 * Cs filter the supply, which sags under load.
 */
struct TubeCircuit
{
	Triode tube;
	double bPlus;			//!< V at the plate resistor, idle
	double plateResistor;
	double cathodeResistor;
	double cathodeCapacitor;
	double gridStopper = 68e3;
	double inputCapacitor = 22e-9;
	double gridLeak = 1e6;
	double outputCapacitor = 22e-9;
	double load = 1e6;
	double supplyResistor = 10e3;
	double supplyCapacitor = 22e-6;
};

/**
 * What a TubeCircuit does, worked out once: its operating point, its gain, where it starts to clip, and its static
 * curves (with the cathode at its idle voltage, as its capacitor holds it, and the plate on its AC load line).
 */
class TubeModel
{
public:
	explicit TubeModel(const TubeCircuit& circuit);

	const TubeCircuit circuit;
	double cathode = 0.0;		//!< idle voltages and current
	double plate = 0.0;
	double current = 0.0;
	double supply = 0.0;		//!< before the supply resistor
	double gain = 0.0;			//!< midband, from the input to the output (negative: the stage inverts)
	double headroom = 0.0;		//!< input peak at which it starts to clip, on whichever side comes first (V)

	//! The static curves for an input voltage: the plate's AC voltage, the plate current and the grid current
	struct Point { float plate, plateCurrent, gridCurrent; };
	auto at(float input) const -> Point;

private:
	auto solve(double input) const -> Point;

	static constexpr int TableSize = 8193;
	static constexpr float TableRange = 60.f; // V, either side
	std::vector<std::array<float, 3>> m_table;
};

/**
 * A tube stage at a given (oversampled) rate, in either of two modes:
 * - light: the static curve, with what the circuit's memory does most audibly approximated around it: the Miller
 *   capacitance as a low-pass at the grid, the input capacitor charged by the grid current (blocking), and the supply
 *   sagging with the average plate current;
 * - circuit: the whole circuit solved at each sample (nodal analysis with trapezoidal capacitors, the DK method:
 *   Newton's method on the two nonlinear currents only).
 * Both take the input in volts and give the stage's output in volts, around 0.
 */
class TubeStage
{
public:
	void setUp(const TubeModel* model, double sampleRate);
	//! Back to the operating point
	void reset();

	auto processLight(float input) -> float;
	auto processCircuit(float input) -> float;

private:
	static constexpr int Nodes = 6; // input coupling, grid, cathode, plate, B+, output
	enum Node { Coupling, Grid, Cathode, Plate, BPlus, Output };

	struct Capacitor
	{
		int a, b;		//!< nodes, -1 being ground
		double g = 0.0;	//!< 2C/T
		double j = 0.0;	//!< history
	};

	auto voltage(const std::array<double, Nodes>& v, int node) const -> double { return node < 0 ? 0.0 : v[node]; }

	const TubeModel* m_model = nullptr;
	double m_period = 0.0;

	// Light
	float m_millerCoefficient = 1.f;
	float m_miller = 0.f;
	float m_blocking = 0.f;		//!< V the input capacitor holds from the grid current
	float m_blockingCharge = 0.f, m_blockingLeak = 0.f;
	float m_averageCurrent = 0.f;
	float m_sagCoefficient = 0.f;

	// Circuit
	std::array<std::array<double, Nodes>, Nodes> m_inverse{};	//!< of the linear part's matrix
	std::array<std::array<double, 2>, Nodes> m_nonlinear{};		//!< the inverse times the currents' incidence
	std::array<std::array<double, 2>, 2> m_k{};					//!< how the currents move the controlling voltages
	std::array<Capacitor, 5> m_capacitors{};	//!< but the input capacitor
	double m_inputConductance = 0.0;	//!< of the input capacitor, whose other side is the input
	double m_inputHistory = 0.0;
	std::array<double, Nodes> m_voltages{};
	double m_vgk = 0.0, m_vpk = 0.0;	//!< of the last sample, Newton's starting point
};

} // namespace lmms

#endif // LMMS_TUBE_STAGE_H
