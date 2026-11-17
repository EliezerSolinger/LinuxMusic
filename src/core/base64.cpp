/*
 * base64.cpp - namespace base64 with methods for encoding/decoding binary data
 *              to/from base64
 *
 * Copyright (c) 2006-2008 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 *
 * This file is part of LMMS - https://lmms.io
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

#include "base64.h"

#include <QBuffer>
#include <QDataStream>

#include "DeprecationHelper.h"

namespace lmms::base64
{


QVariant decode(const QString& b64, QMetaType::Type forceType)
{
	auto ba = QByteArray::fromBase64(b64.toUtf8());
	QBuffer buf( &ba );
	buf.open( QBuffer::ReadOnly );
	QDataStream in( &buf );
	QVariant ret;
	in >> ret;
	if (forceType != QMetaType::UnknownType && typeId(ret) != forceType)
	{
		buf.reset();
		in.setVersion( QDataStream::Qt_3_3 );
		in >> ret;
	}
	return( ret );
}


} // namespace lmms::base64
