/*
 * PluginCategoriesTest.cpp
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

#include <QtTest>

#include "ConfigManager.h"
#include "Engine.h"
#include "PluginCategories.h"
#include "PluginFactory.h"

using namespace lmms;

/**
 * The categories of the built-in plugins (the user's requests of 17/11): Vibed among the synthesizers, the
 * synthesizers in subcategories, and every built-in instrument somewhere (a new one without a category would land
 * in "Other").
 */
class PluginCategoriesTest : public QObject
{
	Q_OBJECT

	static auto descriptor(const char* name) -> const Plugin::Descriptor*
	{
		for (const auto* desc : getPluginFactory()->descriptors(Plugin::Type::Instrument))
		{
			if (QString::fromUtf8(desc->name) == name) { return desc; }
		}
		return nullptr;
	}

private slots:
	void initTestCase() { Engine::init(true); }
	void cleanupTestCase() { Engine::destroy(); }

	void vibedIsASynthesizer()
	{
		const auto* vibed = descriptor("vibedstrings");
		if (!vibed) { QSKIP("Vibed not built here"); }
		QCOMPARE(PluginCategories::categoryOf(vibed), QString{"Synthesizers"});
		QCOMPARE(PluginCategories::subcategoryOf(vibed), QString{"Strings"});
	}

	void everySynthesizerHasASubcategory()
	{
		const auto subcategories = PluginCategories::subcategories("Synthesizers");
		QVERIFY(subcategories.size() >= 3);
		for (const auto* desc : getPluginFactory()->descriptors(Plugin::Type::Instrument))
		{
			if (PluginCategories::categoryOf(desc) != "Synthesizers") { continue; }
			QVERIFY2(subcategories.contains(PluginCategories::subcategoryOf(desc)), desc->name);
		}
		// Only the synthesizers have them
		QVERIFY(PluginCategories::subcategories("Samplers").isEmpty());
	}

	void everyBuiltInInstrumentHasACategory()
	{
		// (the hosts of plugins from elsewhere have theirs too: "Plugin hosts")
		for (const auto* desc : getPluginFactory()->descriptors(Plugin::Type::Instrument))
		{
			const auto name = QString::fromUtf8(desc->name);
			if (name == "lv2instrument") { continue; } // (its plugins are listed by their host)
			QVERIFY2(!PluginCategories::categoryOf(desc).isEmpty(), desc->name);
		}
	}
};

QTEST_GUILESS_MAIN(PluginCategoriesTest)
#include "PluginCategoriesTest.moc"
