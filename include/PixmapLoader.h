/*
 * PixmapLoader.h
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

#ifndef LMMS_PIXMAP_LOADER_H
#define LMMS_PIXMAP_LOADER_H

#include <string>

#include <QPixmap>

#include "lmms_export.h"
#ifdef PLUGIN_NAME
#include "LmmsCommonMacros.h"
#endif

namespace lmms
{

/**
 * The name of an image of the theme (a model's icon, a plugin's logo), for the GUI to draw. Models in the core hold
 * these; only the GUI turns one into a pixmap (pixmap() is the GUI's code, see embed.h), so the core never makes a
 * pixmap, which it cannot do without a GUI.
 */
class LMMS_EXPORT PixmapLoader
{
public:
	PixmapLoader() = default;

	explicit PixmapLoader(std::string name, const char* const* xpm = nullptr) :
		m_name{std::move(name)},
		m_xpm{xpm}
	{ }

	virtual ~PixmapLoader() = default;

	//! The image, from the GUI's pixmap cache (GUI thread only)
	auto pixmap(int width = -1, int height = -1) const -> QPixmap;

	auto pixmapName() const -> const std::string& { return m_name; }

private:
	std::string m_name;
	const char* const* m_xpm = nullptr;
};

#ifdef PLUGIN_NAME

class PluginPixmapLoader : public PixmapLoader
{
public:
	PluginPixmapLoader() = default;

	explicit PluginPixmapLoader(std::string name, const char* const* xpm = nullptr) :
		PixmapLoader{LMMS_STRINGIFY(PLUGIN_NAME) "/" + name, xpm}
	{ }
};

#endif // PLUGIN_NAME

} // namespace lmms

#endif // LMMS_PIXMAP_LOADER_H
