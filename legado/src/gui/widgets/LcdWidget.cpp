/*
 * LcdWidget.cpp - a widget for displaying numbers in LCD style
 *
 * Copyright (c) 2005-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 * Copyright (c) 2008 Paul Giblock <pgllama/at/gmail.com>
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



#include <QStyleOptionFrame>
#include <QPainter>

#include "LcdWidget.h"
#include "DeprecationHelper.h"
#include "embed.h"
#include "FontHelper.h"


namespace lmms::gui
{

LcdWidget::LcdWidget(QWidget* parent, const QString& name, bool leadingZero) :
	LcdWidget(1, parent, name, leadingZero)
{
}




LcdWidget::LcdWidget(int numDigits, QWidget* parent, const QString& name, bool leadingZero) :
	LcdWidget(numDigits, QString("19green"), parent, name, leadingZero)
{
}




LcdWidget::LcdWidget(int numDigits, const QString& style, QWidget* parent, const QString& name, bool leadingZero) :
	QWidget( parent ),
	m_label(),
	m_textColor( 255, 255, 255 ),
	m_textShadowColor( 64, 64, 64 ),
	m_numDigits(numDigits),
	m_seamlessLeft(false),
	m_seamlessRight(false),
	m_leadingZero(leadingZero)
{
	initUi( name, style );
}

void LcdWidget::setValue(int value)
{
	QString s = m_textForValue[value];
	if (s.isEmpty())
	{
		s = QString::number(value);
		if (m_leadingZero)
		{
			s = s.rightJustified(m_numDigits, '0');
		}
	}

	if (m_display != s)
	{
		m_display = s;

		update();
	}
}

void LcdWidget::setValue(float value)
{
	if (-1 < value && value < 0)
	{
		QString s = QString::number(static_cast<int>(value));
		s.prepend('-');
		
		if (m_display != s)
		{
			m_display = s;
			update();
		}
	}
	else
	{
		setValue(static_cast<int>(value));
	}
}




QColor LcdWidget::textColor() const
{
	return m_textColor;
}

void LcdWidget::setTextColor( const QColor & c )
{
	m_textColor = c;
}




QColor LcdWidget::textShadowColor() const
{
	return m_textShadowColor;
}

void LcdWidget::setTextShadowColor( const QColor & c )
{
	m_textShadowColor = c;
}




void LcdWidget::paintEvent( QPaintEvent* )
{
	QPainter p( this );

	QSize cellSize( m_cellWidth, m_cellHeight );

	QRect cellRect( 0, 0, m_cellWidth, m_cellHeight );

	int margin = 1;  // QStyle::PM_DefaultFrameWidth;
	//int lcdWidth = m_cellWidth * m_numDigits + (margin*m_marginWidth)*2;

//	p.translate( width() / 2 - lcdWidth / 2, 0 ); 
	p.save();

	// Don't skip any space and don't draw margin on the left side in seamless mode
	if (m_seamlessLeft)
	{
		p.translate(0, margin);
	}
	else
	{
		p.translate(margin, margin);
		// Left Margin
		p.drawPixmap(cellRect, m_lcdPixmap,
			QRect(QPoint(charsPerPixmap * m_cellWidth, isEnabled() ? 0 : m_cellHeight), cellSize));

		p.translate(m_marginWidth, 0);
	}

	// Padding
	for( int i=0; i < m_numDigits - m_display.length(); i++ ) 
	{
		p.drawPixmap(cellRect, m_lcdPixmap, QRect(QPoint(10 * m_cellWidth, isEnabled() ? 0 : m_cellHeight), cellSize));
		p.translate( m_cellWidth, 0 );
	}

	// Digits
	for (const auto& digit : m_display)
	{
		int val = digit.digitValue();
		if( val < 0 ) 
		{
			if (digit == '-') val = 11;
			else
				val = 10;
		}
		p.drawPixmap(cellRect, m_lcdPixmap, QRect(QPoint(val * m_cellWidth, isEnabled() ? 0 : m_cellHeight), cellSize));
		p.translate( m_cellWidth, 0 );
	}

	// Right Margin
	p.drawPixmap(QRect(0, 0, m_seamlessRight ? 0 : m_marginWidth - 1, m_cellHeight), m_lcdPixmap,
		QRect(charsPerPixmap * m_cellWidth, isEnabled() ? 0 : m_cellHeight, m_cellWidth / 2, m_cellHeight));

	p.restore();

	// Border
	// When either the left or right edge is seamless, the border drawing must be done
	// by the encapsulating class (usually LcdFloatSpinBox).
	if (!m_seamlessLeft && !m_seamlessRight)
	{
		QStyleOptionFrame opt;
		opt.initFrom(this);
		opt.state = QStyle::State_Sunken;
		opt.rect = QRect(0, 0, m_cellWidth * m_numDigits + (margin + m_marginWidth) * 2 - 1,
			m_cellHeight + (margin * 2));

		style()->drawPrimitive(QStyle::PE_Frame, &opt, &p, this);
	}

	p.resetTransform();

	// Label
	if( !m_label.isEmpty() )
	{
		p.setFont(adjustedToPixelSize(p.font(), DEFAULT_FONT_SIZE));
		p.setPen( textShadowColor() );
		p.drawText(width() / 2 -
				p.fontMetrics().horizontalAdvance(m_label) / 2 + 1,
						height(), m_label);
		p.setPen( textColor() );
		p.drawText(width() / 2 -
				p.fontMetrics().horizontalAdvance(m_label) / 2,
						height() - 1, m_label);
	}

}




void LcdWidget::setLabel( const QString& label )
{
	m_label = label;
	updateSize();
}




void LcdWidget::setMarginWidth( int width )
{
	m_marginWidth = width;

	updateSize();
}




void LcdWidget::updateSize()
{
	const int marginX1 = m_seamlessLeft ? 0 : 1 + m_marginWidth;
	const int marginX2 = m_seamlessRight ? 0 : 1 + m_marginWidth;
	const int marginY = 1;
	if (m_label.isEmpty())
	{
		setFixedSize(
			m_cellWidth * m_numDigits + marginX1 + marginX2,
			m_cellHeight + (2 * marginY)
		);
	}
	else
	{
		setFixedSize(
			qMax<int>(
				m_cellWidth * m_numDigits + marginX1 + marginX2,
				QFontMetrics(adjustedToPixelSize(font(), DEFAULT_FONT_SIZE)).horizontalAdvance(m_label)
			),
			m_cellHeight + (2 * marginY) + 9
		);
	}

	update();
}




void LcdWidget::initUi(const QString& name , const QString& style)
{
	setEnabled( true );

	setWindowTitle( name );

	// We should make a factory for these or something.
	//m_lcdPixmap = embed::getIconPixmap(QString("lcd_" + style).toUtf8().constData());
	//m_lcdPixmap = embed::getIconPixmap("lcd_19green"); // TODO!!

	m_style = style;
	loadPixmap();
}




void LcdWidget::loadPixmap()
{
	m_lcdPixmap = embed::getIconPixmap(QString("lcd_" + m_style).toUtf8().constData());
	m_cellWidth = m_lcdPixmap.size().width() / LcdWidget::charsPerPixmap;
	m_cellHeight = m_lcdPixmap.size().height() / 2;
	m_marginWidth =  m_cellWidth / 2;

	if (m_flatDigits)
	{
		// Same layout as the images (digits 0-9, blank, minus, then the margin; enabled row above the
		// disabled row), drawn with the widget's font so that every LCD keeps its size
		const auto size = m_lcdPixmap.size();
		QPixmap flat(size);
		flat.fill(m_flatBackground);

		QColor color(43, 212, 168);
		if (m_style.contains("red")) { color = QColor(232, 84, 72); }
		else if (m_style.contains("purple")) { color = QColor(155, 140, 255); }
		else if (m_style.contains("pink")) { color = QColor(255, 111, 174); }

		QFont digitFont = font();
		digitFont.setBold(true);
		digitFont.setPixelSize(std::max(6, m_cellHeight * 4 / 5));
		while (digitFont.pixelSize() > 6 && QFontMetrics(digitFont).horizontalAdvance('0') > m_cellWidth)
		{
			digitFont.setPixelSize(digitFont.pixelSize() - 1);
		}

		QPainter p(&flat);
		p.setRenderHint(QPainter::TextAntialiasing);
		p.setFont(digitFont);
		const QString chars = "0123456789 -";
		for (int row = 0; row < 2; ++row)
		{
			auto rowColor = color;
			if (row == 1) { rowColor.setAlpha(110); }
			p.setPen(rowColor);
			for (int i = 0; i < chars.size(); ++i)
			{
				p.drawText(QRect(i * m_cellWidth, row * m_cellHeight, m_cellWidth, m_cellHeight),
					Qt::AlignCenter, chars.mid(i, 1));
			}
		}
		m_lcdPixmap = flat;
	}

	updateSize();
}




void LcdWidget::setFlatDigits(bool flat)
{
	if (m_flatDigits == flat) { return; }
	m_flatDigits = flat;
	loadPixmap();
	update();
}




void LcdWidget::setFlatBackground(const QColor& color)
{
	m_flatBackground = color;
	if (m_flatDigits)
	{
		loadPixmap();
		update();
	}
}

} // namespace lmms::gui
