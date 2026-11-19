/*
 * TubeStage.cpp - a triode preamp stage: its circuit, solved per sample or approximated around its static curve
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

#include "TubeStage.h"

#include <algorithm>
#include <cmath>
#include <numbers>

namespace lmms
{

namespace
{

//! log(1 + e^x), without overflow, and its derivative, the logistic function
auto softplus(double x) -> double { return x > 30.0 ? x : std::log1p(std::exp(x)); }
auto logistic(double x) -> double { return 1.0 / (1.0 + std::exp(-x)); }

//! Finds where an increasing function crosses 0 between @p low and @p high
template<class F>
auto bisect(F f, double low, double high, int iterations = 56) -> double
{
	for (int i = 0; i < iterations; ++i)
	{
		const double middle = (low + high) / 2.0;
		(f(middle) < 0.0 ? low : high) = middle;
	}
	return (low + high) / 2.0;
}

// Dempwolf and Zölzer's grid current, with their parameters for the 12AX7 (its constant leakage left out: it would
// only offset the grid)
constexpr double GridConductance = 6.06e-4;
constexpr double GridExponent = 1.354;
constexpr double GridSharpness = 13.9;

} // namespace




auto Triode::plateCurrent(double vgk, double vpk) const -> Current
{
	if (vpk <= 0.0) { return {0.0, 0.0, 0.0}; }
	const double root = std::sqrt(kvb + vpk * vpk);
	const double x = kp * (1.0 / mu + vgk / root);
	const double soft = softplus(x);
	const double e1 = vpk / kp * soft;
	if (e1 <= 1e-12) { return {0.0, 0.0, 0.0}; }
	const double sigma = logistic(x);
	const double power = std::pow(e1, ex - 1.0);
	const double value = 2.0 * power * e1 / kg1;
	const double byE1 = 2.0 * ex * power / kg1;
	const double e1ByGrid = vpk * sigma / root;
	const double e1ByPlate = soft / kp - vpk * vpk * vgk * sigma / (root * root * root);
	return {value, byE1 * e1ByGrid, byE1 * e1ByPlate};
}




auto Triode::gridCurrent(double vgk) const -> Current
{
	const double x = GridSharpness * vgk;
	const double soft = softplus(x) / GridSharpness;
	if (soft <= 1e-300) { return {0.0, 0.0, 0.0}; }
	const double power = std::pow(soft, GridExponent - 1.0);
	return {GridConductance * power * soft, GridConductance * GridExponent * power * logistic(x), 0.0};
}




TubeModel::TubeModel(const TubeCircuit& circuit)
	: circuit{circuit}
{
	const auto& c = circuit;

	// The operating point: the current through the cathode resistor biases the grid
	current = bisect([&c](double i) {
		return i - c.tube.plateCurrent(-i * c.cathodeResistor, c.bPlus - i * (c.plateResistor + c.cathodeResistor)).value;
	}, 0.0, c.bPlus / (c.plateResistor + c.cathodeResistor));
	cathode = current * c.cathodeResistor;
	plate = c.bPlus - current * c.plateResistor;
	supply = c.bPlus + current * c.supplyResistor;

	m_table.resize(TableSize);
	for (int i = 0; i < TableSize; ++i)
	{
		const double input = -TableRange + 2.0 * TableRange * i / (TableSize - 1);
		const auto point = solve(input);
		m_table[i] = {point.plate, point.plateCurrent, point.gridCurrent};
	}

	constexpr float step = 1e-3f;
	gain = (at(step).plate - at(-step).plate) / (2.0 * step);

	// Where it starts to clip: where its slope has fallen to half its midband slope, on either side
	auto clipsAt = [this](float direction) {
		constexpr float step = 2.f * TableRange / (TableSize - 1);
		for (float input = step; input < TableRange; input += step)
		{
			const float slope = (at(direction * (input + step)).plate - at(direction * input).plate) / (direction * step);
			if (std::abs(slope) < 0.5 * std::abs(gain)) { return input; }
		}
		return TableRange;
	};
	headroom = std::min(clipsAt(1.f), clipsAt(-1.f));
}




auto TubeModel::solve(double input) const -> Point
{
	const auto& c = circuit;
	// The grid: the input, less what the grid current drops across the grid stopper (the input capacitor passes
	// the signal, the cathode's capacitor holds it at its idle voltage)
	const double grid = bisect([&](double vg) {
		return vg + c.gridStopper * c.tube.gridCurrent(vg - cathode).value - input;
	}, std::min(input, cathode) - 2.0, input);
	const double vgk = grid - cathode;

	// The plate, on the AC load line: the plate resistor and the load, through the output capacitor
	const double load = c.plateResistor * c.load / (c.plateResistor + c.load);
	const double vp = bisect([&](double v) {
		return v - plate + load * (c.tube.plateCurrent(vgk, v - cathode).value - current);
	}, cathode, plate + load * current);

	return {static_cast<float>(vp - plate), static_cast<float>(c.tube.plateCurrent(vgk, vp - cathode).value),
		static_cast<float>(c.tube.gridCurrent(vgk).value)};
}




auto TubeModel::at(float input) const -> Point
{
	const float position = (input + TableRange) / (2.f * TableRange) * (TableSize - 1);
	if (position <= 0.f) { const auto& p = m_table.front(); return {p[0], p[1], p[2]}; }
	if (position >= TableSize - 1) { const auto& p = m_table.back(); return {p[0], p[1], p[2]}; }
	const auto i = static_cast<std::size_t>(position);
	const float f = position - static_cast<float>(i);
	const auto& a = m_table[i];
	const auto& b = m_table[i + 1];
	return {a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f};
}




void TubeStage::setUp(const TubeModel* model, double sampleRate)
{
	m_model = model;
	m_period = 1.0 / sampleRate;
	const auto& c = model->circuit;
	const double t = m_period;

	// Light
	const double miller = c.tube.gridToCathode + c.tube.gridToPlate * (1.0 + std::abs(model->gain));
	const double millerCutoff = 1.0 / (2.0 * std::numbers::pi * c.gridStopper * miller);
	m_millerCoefficient = static_cast<float>(1.0 - std::exp(-2.0 * std::numbers::pi * millerCutoff * t));
	m_blockingCharge = static_cast<float>(t / c.inputCapacitor);
	m_blockingLeak = static_cast<float>(t / (c.gridLeak * c.inputCapacitor));
	m_sagCoefficient = static_cast<float>(1.0 - std::exp(-t / (c.supplyResistor * c.supplyCapacitor)));

	// Circuit: the linear part's matrix, with the capacitors as conductances (trapezoidal rule)
	auto g = [t](double capacitance) { return 2.0 * capacitance / t; };
	m_inputConductance = g(c.inputCapacitor);
	m_capacitors = {{
		{Grid, Cathode, g(c.tube.gridToCathode)},
		{Grid, Plate, g(c.tube.gridToPlate)},
		{Cathode, -1, g(c.cathodeCapacitor)},
		{BPlus, -1, g(c.supplyCapacitor)},
		{Plate, Output, g(c.outputCapacitor)},
	}};
	auto a = std::array<std::array<double, Nodes>, Nodes>{};
	auto conductance = [&a](int n1, int n2, double value) {
		if (n1 >= 0) { a[n1][n1] += value; }
		if (n2 >= 0) { a[n2][n2] += value; }
		if (n1 >= 0 && n2 >= 0) { a[n1][n2] -= value; a[n2][n1] -= value; }
	};
	conductance(Coupling, -1, m_inputConductance); // to the input, a known voltage
	conductance(Coupling, -1, 1.0 / c.gridLeak);
	conductance(Coupling, Grid, 1.0 / c.gridStopper);
	conductance(Cathode, -1, 1.0 / c.cathodeResistor);
	conductance(Plate, BPlus, 1.0 / c.plateResistor);
	conductance(BPlus, -1, 1.0 / c.supplyResistor); // to the supply, a known voltage
	conductance(Output, -1, 1.0 / c.load);
	for (const auto& capacitor : m_capacitors) { conductance(capacitor.a, capacitor.b, capacitor.g); }

	// Its inverse (Gauss-Jordan with partial pivoting)
	auto inverse = std::array<std::array<double, Nodes>, Nodes>{};
	for (int i = 0; i < Nodes; ++i) { inverse[i][i] = 1.0; }
	for (int col = 0; col < Nodes; ++col)
	{
		int pivot = col;
		for (int row = col + 1; row < Nodes; ++row)
		{
			if (std::abs(a[row][col]) > std::abs(a[pivot][col])) { pivot = row; }
		}
		std::swap(a[col], a[pivot]);
		std::swap(inverse[col], inverse[pivot]);
		const double scale = 1.0 / a[col][col];
		for (int k = 0; k < Nodes; ++k) { a[col][k] *= scale; inverse[col][k] *= scale; }
		for (int row = 0; row < Nodes; ++row)
		{
			if (row == col) { continue; }
			const double factor = a[row][col];
			for (int k = 0; k < Nodes; ++k) { a[row][k] -= factor * a[col][k]; inverse[row][k] -= factor * inverse[col][k]; }
		}
	}
	m_inverse = inverse;

	// The nonlinear currents: the grid current from the grid to the cathode, the plate current from the plate to the
	// cathode
	for (int n = 0; n < Nodes; ++n)
	{
		m_nonlinear[n][0] = m_inverse[n][Cathode] - m_inverse[n][Grid];
		m_nonlinear[n][1] = m_inverse[n][Cathode] - m_inverse[n][Plate];
	}
	for (int i = 0; i < 2; ++i)
	{
		m_k[0][i] = m_nonlinear[Grid][i] - m_nonlinear[Cathode][i];
		m_k[1][i] = m_nonlinear[Plate][i] - m_nonlinear[Cathode][i];
	}
	reset();
}




void TubeStage::reset()
{
	const auto& m = *m_model;
	m_miller = 0.f;
	m_blocking = 0.f;
	m_averageCurrent = static_cast<float>(m.current);

	m_voltages = {0.0, 0.0, m.cathode, m.plate, m.circuit.bPlus, 0.0};
	for (auto& capacitor : m_capacitors)
	{
		capacitor.j = capacitor.g * (voltage(m_voltages, capacitor.a) - voltage(m_voltages, capacitor.b));
	}
	m_inputHistory = 0.0;
	m_vgk = -m.cathode;
	m_vpk = m.plate - m.cathode;
}




auto TubeStage::processLight(float input) -> float
{
	const auto& m = *m_model;
	m_miller += (input - m_miller) * m_millerCoefficient;
	// The supply sags as the average plate current rises; the stage's curves scale with it
	const float sag = static_cast<float>(m.circuit.supplyResistor / m.circuit.bPlus)
		* (m_averageCurrent - static_cast<float>(m.current));
	const float supply = std::clamp(1.f - sag, 0.5f, 1.5f);
	const auto point = m.at((m_miller - m_blocking) / supply);
	m_blocking += point.gridCurrent * m_blockingCharge - m_blocking * m_blockingLeak;
	m_averageCurrent += (point.plateCurrent - m_averageCurrent) * m_sagCoefficient;
	return point.plate * supply;
}




auto TubeStage::processCircuit(float input) -> float
{
	const auto& c = m_model->circuit;
	// The linear part's sources: the input through its capacitor, the supply, the capacitors' histories
	auto b = std::array<double, Nodes>{};
	b[Coupling] += m_inputConductance * input - m_inputHistory;
	b[BPlus] += m_model->supply / c.supplyResistor;
	for (const auto& capacitor : m_capacitors)
	{
		if (capacitor.a >= 0) { b[capacitor.a] += capacitor.j; }
		if (capacitor.b >= 0) { b[capacitor.b] -= capacitor.j; }
	}
	auto v0 = std::array<double, Nodes>{};
	for (int n = 0; n < Nodes; ++n)
	{
		double sum = 0.0;
		for (int k = 0; k < Nodes; ++k) { sum += m_inverse[n][k] * b[k]; }
		v0[n] = sum;
	}
	const double p0g = v0[Grid] - v0[Cathode];
	const double p0p = v0[Plate] - v0[Cathode];

	// Newton's method on the controlling voltages
	double vgk = m_vgk, vpk = m_vpk;
	Triode::Current ig{}, ip{};
	for (int iteration = 0; iteration < 40; ++iteration)
	{
		ig = c.tube.gridCurrent(vgk);
		ip = c.tube.plateCurrent(vgk, vpk);
		const double f0 = vgk - p0g - (m_k[0][0] * ig.value + m_k[0][1] * ip.value);
		const double f1 = vpk - p0p - (m_k[1][0] * ig.value + m_k[1][1] * ip.value);
		const double j00 = 1.0 - (m_k[0][0] * ig.byGrid + m_k[0][1] * ip.byGrid);
		const double j01 = -m_k[0][1] * ip.byPlate;
		const double j10 = -(m_k[1][0] * ig.byGrid + m_k[1][1] * ip.byGrid);
		const double j11 = 1.0 - m_k[1][1] * ip.byPlate;
		const double determinant = j00 * j11 - j01 * j10;
		if (determinant == 0.0) { break; }
		const double dg = std::clamp((f0 * j11 - f1 * j01) / determinant, -1.0, 1.0);
		const double dp = std::clamp((f1 * j00 - f0 * j10) / determinant, -50.0, 50.0);
		vgk -= dg;
		vpk -= dp;
		if (std::abs(dg) < 1e-9 && std::abs(dp) < 1e-7) { break; }
	}
	ig = c.tube.gridCurrent(vgk);
	ip = c.tube.plateCurrent(vgk, vpk);
	m_vgk = vgk;
	m_vpk = vpk;

	for (int n = 0; n < Nodes; ++n) { m_voltages[n] = v0[n] + m_nonlinear[n][0] * ig.value + m_nonlinear[n][1] * ip.value; }

	// The capacitors' histories for the next sample
	for (auto& capacitor : m_capacitors)
	{
		const double across = voltage(m_voltages, capacitor.a) - voltage(m_voltages, capacitor.b);
		capacitor.j = 2.0 * capacitor.g * across - capacitor.j;
	}
	m_inputHistory = 2.0 * m_inputConductance * (input - m_voltages[Coupling]) - m_inputHistory;

	return static_cast<float>(m_voltages[Output]);
}

} // namespace lmms
